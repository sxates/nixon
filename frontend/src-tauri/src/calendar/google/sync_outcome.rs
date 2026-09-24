//! What a Google Calendar sync pass reports, and the gate every pass goes
//! through (specs/0074 W1, specs/0075 W1a).
//!
//! The rule this module exists for: a pass that didn't run can never report
//! `Synced`. Before 0075 every skip path (the `invalid_grant` latch, a
//! coalesced trigger, a stalled lock holder) returned `Ok(())`, which the UI
//! showed as "synced" while nothing had run for days.
//!
//! [`gated_pass`] is the gate: latch → [`SyncOutcome::AuthRequired`]; lock busy
//! → [`SyncOutcome::AlreadyRunning`] (manual triggers wait up to
//! [`MANUAL_WAIT`] first); and the pass itself is capped at [`PASS_CAP`] so no
//! single request can hold the lock forever.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// How long a click someone is waiting on (Sync now / connect / selection)
/// waits for an in-flight pass before reporting `AlreadyRunning`.
pub(super) const MANUAL_WAIT: Duration = Duration::from_secs(30);
/// Hard cap on one pass's event phase (token + calendar list + events).
pub(super) const PASS_CAP: Duration = Duration::from_secs(120);

/// Who asked for the pass. Only affects lock behaviour and the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTrigger {
    Manual,
    Connect,
    Selection,
    Focus,
    Agenda,
    Upcoming,
    Prep,
    Timer,
}

impl SyncTrigger {
    /// Triggers a person is waiting on wait for the lock; the rest coalesce.
    fn waits_for_lock(self) -> bool {
        matches!(self, Self::Manual | Self::Connect | Self::Selection)
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Connect => "connect",
            Self::Selection => "selection",
            Self::Focus => "focus",
            Self::Agenda => "agenda",
            Self::Upcoming => "upcoming",
            Self::Prep => "prep",
            Self::Timer => "timer",
        }
    }
}

/// How one calendar was synced this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncMode {
    Incremental,
    Full,
    /// The syncToken expired server-side (410 GONE).
    FullAfterGone,
    /// A recurring series master turned up in an incremental page (specs/0054 W5).
    FullAfterSeriesChange,
}

impl SyncMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Incremental => "incremental",
            Self::Full => "full",
            Self::FullAfterGone => "full(gone)",
            Self::FullAfterSeriesChange => "full(series-change)",
        }
    }
}

/// One calendar's result within a `Synced` pass.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSyncReport {
    /// Never logged: for the primary calendar it IS the account email.
    pub calendar_id: String,
    pub summary: String,
    pub is_primary: bool,
    pub mode: SyncMode,
    /// Items Google returned.
    pub fetched: usize,
    pub upserted: usize,
    pub deleted: usize,
    pub duration_ms: u64,
    /// This calendar failed (the cache keeps serving); the pass still counts.
    pub error: Option<String>,
}

/// The result of one sync request. Serialized for the frontend as
/// `{ kind: "synced", calendars, durationMs } | { kind: "alreadyRunning" } | …`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncOutcome {
    /// The pass ran. Per-calendar failures are reported here, not as an `Err`.
    #[serde(rename_all = "camelCase")]
    Synced {
        calendars: Vec<CalendarSyncReport>,
        duration_ms: u64,
    },
    /// Another pass holds the lock (a background trigger, or a manual one that
    /// waited [`MANUAL_WAIT`] and gave up).
    AlreadyRunning,
    /// The grant lapsed (`invalid_grant`); the user must reconnect.
    AuthRequired,
    NotConnected,
    NotConfigured,
    /// The demo dataset is active (debug builds only).
    Suppressed,
    /// Connected, but no calendar is selected.
    NoCalendarsSelected,
}

impl SyncOutcome {
    /// Rows written or removed across every calendar — drives
    /// `google-calendar-synced`.
    pub fn changed(&self) -> usize {
        match self {
            Self::Synced { calendars, .. } => {
                calendars.iter().map(|c| c.upserted + c.deleted).sum()
            }
            _ => 0,
        }
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Synced { .. } => "synced",
            Self::AlreadyRunning => "already-running",
            Self::AuthRequired => "auth-required",
            Self::NotConnected => "not-connected",
            Self::NotConfigured => "not-configured",
            Self::Suppressed => "suppressed",
            Self::NoCalendarsSelected => "no-calendars-selected",
        }
    }
}

/// The most recent `Synced` pass, so the status command can show each
/// calendar's last error without a migration.
static LAST_REPORT: Mutex<Option<(DateTime<Utc>, SyncOutcome)>> = Mutex::new(None);

pub(super) fn remember(outcome: &SyncOutcome) {
    if matches!(outcome, SyncOutcome::Synced { .. }) {
        let mut guard = LAST_REPORT.lock().unwrap_or_else(|p| p.into_inner());
        *guard = Some((Utc::now(), outcome.clone()));
    }
}

/// The last pass's error for `calendar_id`, if that pass failed it.
pub(super) fn last_error_for(calendar_id: &str) -> Option<String> {
    let guard = LAST_REPORT.lock().unwrap_or_else(|p| p.into_inner());
    match guard.as_ref() {
        Some((_, SyncOutcome::Synced { calendars, .. })) => calendars
            .iter()
            .find(|c| c.calendar_id == calendar_id)
            .and_then(|c| c.error.clone()),
        _ => None,
    }
}

/// Run `pass` under `lock` with the latch and the cap applied. `lock` and
/// `latch` are parameters (production passes the `sync.rs` statics) so the
/// tests can use their own and never race each other.
///
/// On a cap expiry the pass future is dropped — which drops the lock guard —
/// and the result is an `Err`, never an outcome that claims the pass ran.
pub(super) async fn gated_pass<Fut>(
    lock: &tokio::sync::Mutex<()>,
    latch: &AtomicBool,
    trigger: SyncTrigger,
    pass: impl FnOnce() -> Fut,
) -> Result<SyncOutcome>
where
    Fut: Future<Output = Result<SyncOutcome>>,
{
    if latch.load(Ordering::SeqCst) {
        return Ok(SyncOutcome::AuthRequired);
    }
    let guard = if trigger.waits_for_lock() {
        tokio::time::timeout(MANUAL_WAIT, lock.lock()).await.ok()
    } else {
        lock.try_lock().ok()
    };
    let Some(_guard) = guard else {
        return Ok(SyncOutcome::AlreadyRunning);
    };
    // The pass we waited behind may have latched it.
    if latch.load(Ordering::SeqCst) {
        return Ok(SyncOutcome::AuthRequired);
    }
    match tokio::time::timeout(PASS_CAP, pass()).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!(
            "Google Calendar sync timed out after {}s; it will retry on the next trigger",
            PASS_CAP.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn synced() -> SyncOutcome {
        SyncOutcome::Synced {
            calendars: Vec::new(),
            duration_ms: 1,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_never_resolving_pass_releases_the_lock_within_the_cap() {
        let lock = tokio::sync::Mutex::new(());
        let latch = AtomicBool::new(false);
        let started = tokio::time::Instant::now();
        // A token refresh stuck on a half-open socket: never resolves.
        let pass = gated_pass(&lock, &latch, SyncTrigger::Timer, || {
            std::future::pending::<Result<SyncOutcome>>()
        });
        let result = tokio::time::timeout(PASS_CAP * 2, pass)
            .await
            .expect("the pass cap must fire, not the test's backstop");
        let err = result.expect_err("a stalled pass must not report an outcome");
        assert!(err.to_string().contains("timed out"), "{err}");
        assert!(started.elapsed() <= PASS_CAP + Duration::from_secs(1));
        assert!(
            lock.try_lock().is_ok(),
            "the lock must be free after the cap"
        );
        // And the next manual pass actually runs.
        let next = gated_pass(&lock, &latch, SyncTrigger::Manual, || async {
            Ok(synced())
        })
        .await
        .unwrap();
        assert!(matches!(next, SyncOutcome::Synced { .. }));
    }

    #[tokio::test]
    async fn the_latch_returns_auth_required_and_never_runs_the_pass() {
        let lock = tokio::sync::Mutex::new(());
        let latch = AtomicBool::new(true);
        let ran = AtomicUsize::new(0);
        for trigger in [SyncTrigger::Manual, SyncTrigger::Timer] {
            let out = gated_pass(&lock, &latch, trigger, || async {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(synced())
            })
            .await
            .unwrap();
            assert!(matches!(out, SyncOutcome::AuthRequired), "{out:?}");
        }
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_coalesced_background_trigger_returns_already_running() {
        let lock = tokio::sync::Mutex::new(());
        let latch = AtomicBool::new(false);
        let _held = lock.lock().await; // another pass is in flight
        let ran = AtomicUsize::new(0);
        for trigger in [SyncTrigger::Focus, SyncTrigger::Timer, SyncTrigger::Agenda] {
            let out = gated_pass(&lock, &latch, trigger, || async {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok(synced())
            })
            .await
            .unwrap();
            assert!(matches!(out, SyncOutcome::AlreadyRunning), "{out:?}");
        }
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_manual_trigger_waits_for_the_lock_but_not_forever() {
        let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let latch = AtomicBool::new(false);
        // Held for 5s: the manual pass waits and then runs.
        let held = lock.clone().lock_owned().await;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(held);
        });
        let out = gated_pass(&lock, &latch, SyncTrigger::Manual, || async {
            Ok(synced())
        })
        .await
        .unwrap();
        assert!(matches!(out, SyncOutcome::Synced { .. }), "{out:?}");
        // Held forever: the manual pass gives up after MANUAL_WAIT.
        let _stuck = lock.lock().await;
        let pass = gated_pass(&lock, &latch, SyncTrigger::Manual, || async {
            Ok(synced())
        });
        let out = tokio::time::timeout(MANUAL_WAIT * 2, pass)
            .await
            .expect("the manual wait must be bounded")
            .unwrap();
        assert!(matches!(out, SyncOutcome::AlreadyRunning), "{out:?}");
    }

    #[test]
    fn the_wire_shape_is_kind_tagged_camel_case() {
        let out = SyncOutcome::Synced {
            calendars: vec![CalendarSyncReport {
                calendar_id: "c".into(),
                summary: "Work".into(),
                is_primary: true,
                mode: SyncMode::FullAfterSeriesChange,
                fetched: 2,
                upserted: 1,
                deleted: 1,
                duration_ms: 3,
                error: None,
            }],
            duration_ms: 9,
        };
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["kind"], "synced");
        assert_eq!(v["durationMs"], 9);
        assert_eq!(v["calendars"][0]["isPrimary"], true);
        assert_eq!(v["calendars"][0]["mode"], "fullAfterSeriesChange");
        assert_eq!(out.changed(), 2);
        let v = serde_json::to_value(SyncOutcome::AlreadyRunning).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "alreadyRunning" }));
        let v = serde_json::to_value(SyncOutcome::NoCalendarsSelected).unwrap();
        assert_eq!(v["kind"], "noCalendarsSelected");
    }
}
