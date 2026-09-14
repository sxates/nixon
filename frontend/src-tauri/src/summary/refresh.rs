//! specs/0041 WS2 — auto-refresh a speakerless summary once offline diarization lands.
//!
//! The post-stop pipeline races itself: the meeting-details page auto-generates the
//! summary as soon as it loads, while `api_diarize_meeting` runs for minutes in the
//! background. The summary path only prefixes `Name: text` speaker attribution when
//! speakers already exist in the DB, so the auto-summary usually runs unattributed and
//! the 0034 action-item extractor (which reads the summary text) can't resolve owners.
//!
//! This module is the backend-owned trigger (it survives page navigation): at the
//! successful end of the offline diarization pass, re-run the summary through the same
//! path auto-summary uses — but only when ALL of these hold:
//!   1. the pass actually assigned speakers,
//!   2. the meeting's latest summary is COMPLETED and was NOT speaker-attributed
//!      (`summary_processes.speaker_attributed = 0`), and
//!   3. the summary is pristine — the stored `$.markdown` still matches the
//!      fingerprint persisted at generation time (`generated_markdown_hash`). There is
//!      no cheap user-edited flag today (edits rewrite `result` wholesale via
//!      `api_save_meeting_summary`), so the hash comparison IS the edited-signal.
//!      Legacy rows without a hash are conservatively treated as edited.
//!
//! Re-entrancy: the regenerated summary recomputes `speaker_attributed`; since speakers
//! now resolve it persists 1, so a second diarization pass can never loop this trigger.
//!
//! If a summary run is still in flight when diarization finishes (short meeting, slow
//! LLM), that run may have read the transcript BEFORE speakers were persisted — so we
//! wait (bounded) for it to settle and then re-decide once, instead of skipping and
//! leaving the summary permanently speakerless.

use crate::database::repositories::summary::SummaryProcessesRepository;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::summary::cache_key::stable_text_fingerprint;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime};
use tracing::{info, warn};

/// Emitted the moment a post-diarization summary regeneration actually starts (the
/// process row is already reset to PENDING). This is the ONLY signal the meeting page
/// gets on the deferred path (`wait_then_refresh`): by then `diarization-complete`
/// already fired with `summaryRefreshing: false`, so without this event the refresh
/// would be invisible until a manual reload. The immediate path emits it too (just
/// before `diarization-complete`), so the frontend has a single trigger.
pub const EVENT_SUMMARY_REFRESH_STARTED: &str = "summary-refresh-started";

/// How often to re-check an in-flight summary run before re-deciding.
const IN_FLIGHT_POLL_INTERVAL: Duration = Duration::from_secs(15);
/// Give up waiting on an in-flight run after this long (mirrors the frontend's
/// ~16-minute summary polling timeout with headroom).
const IN_FLIGHT_MAX_WAIT: Duration = Duration::from_secs(20 * 60);

/// The trigger decision. Everything except `Refresh` is a reason NOT to regenerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeakerRefreshDecision {
    /// Speakerless, completed, pristine summary + speakers assigned → regenerate.
    Refresh,
    /// The diarization pass assigned no speakers; a re-run would change nothing.
    SkipNoSpeakersAssigned,
    /// No summary process row — the user never generated (or auto-generated) one;
    /// don't mint a summary they didn't ask for.
    SkipNoSummary,
    /// A summary run is currently pending/processing (see module docs — the caller
    /// waits for it to settle and re-decides).
    SkipSummaryInFlight,
    /// The latest run failed/was cancelled; regenerating on top of a failure would
    /// surprise the user — the manual Regenerate affordance covers this.
    SkipSummaryNotCompleted,
    /// The summary already used speaker attribution — nothing to improve (and the
    /// loop guard: refreshed summaries land here).
    SkipAlreadyAttributed,
    /// The stored markdown no longer matches the generation-time fingerprint (or the
    /// row predates the fingerprint column): user edits are never clobbered.
    SkipUserEdited,
    /// specs/0044 WS3: the resolved speaker-name set is identical to what the stored
    /// summary already used — a regeneration would change nothing.
    SkipNamesUnchanged,
}

/// What the trigger actually did at the end of a diarization pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeakerRefreshOutcome {
    /// A regeneration was started (the process row is already reset to PENDING when
    /// this returns, so a `diarization-complete` listener polling `api_get_summary`
    /// can never read the stale 'completed' status first).
    Started,
    /// A summary run was in flight; a background task is waiting for it to settle
    /// and will re-decide (and possibly start a refresh) later.
    DeferredBehindInFlightRun,
    /// No refresh; the decision says why.
    Skipped(SpeakerRefreshDecision),
}

/// The slice of `summary_processes` the decision needs.
#[derive(Debug, Clone)]
pub struct SummaryRefreshSnapshot {
    /// Process status as stored ('PENDING'/'processing'/'completed'/'failed'/...).
    pub status: String,
    /// Whether the stored summary was generated from a speaker-attributed transcript.
    pub speaker_attributed: bool,
    /// Fingerprint of `$.markdown` at generation time; `None` on legacy rows.
    pub generated_markdown_hash: Option<String>,
    /// The current `$.markdown` from the stored result JSON, if parseable.
    pub current_markdown: Option<String>,
    /// specs/0044 WS3: fingerprint of the resolved speaker display-name set at
    /// generation time; `None` on rows from before the 0044 migration (treated as
    /// "names changed" — the markdown pristine guard still protects edits).
    pub speaker_names_hash: Option<String>,
}

/// The generation-time fingerprint of a stored summary markdown — the exact function
/// the completion path persists into `generated_markdown_hash`. Public so integration
/// tests (and any future caller) produce hashes that round-trip with the pristine guard.
pub fn generation_fingerprint(markdown: &str) -> String {
    stable_text_fingerprint(markdown)
}

/// Fingerprint of a resolved speaker display-name SET (specs/0044 WS3): trimmed,
/// blank-dropped, sorted, deduplicated — so segment order, repetition, and
/// whitespace never produce a spurious "names changed". The summary completion
/// path persists this into `summary_processes.speaker_names_hash`; the
/// post-naming trigger recomputes it from the live DB and compares.
pub fn speaker_names_fingerprint<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut set: Vec<&str> = names
        .into_iter()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    set.sort_unstable();
    set.dedup();
    stable_text_fingerprint(&set.join("\n"))
}

/// The CURRENT resolved speaker display-name fingerprint for a meeting — same
/// resolution the summary path uses (`speakers.display_name` LEFT-JOINed onto
/// segments), so it round-trips exactly with what generation persisted.
pub async fn current_speaker_names_hash(pool: &SqlitePool, meeting_id: &str) -> Result<String> {
    let segments = TranscriptsRepository::get_transcript_segments_with_speakers(pool, meeting_id)
        .await
        .with_context(|| format!("load speaker names for meeting {meeting_id}"))?;
    Ok(speaker_names_fingerprint(
        segments.iter().filter_map(|(name, _)| name.as_deref()),
    ))
}

/// One-call attribution loader for the summary service (specs/0044 WS3):
/// `(attributed_text_if_any_speaker_resolved, has_speakers, names_hash)`. The
/// names hash is computed from the SAME segment load that produced the text, so
/// what generation persists is exactly what the post-naming trigger recomputes.
/// A DB error degrades to the plain-transcript path (`None`, `false`, empty-set
/// hash) — never fatal to the summary run.
pub async fn load_attribution(
    pool: &SqlitePool,
    meeting_id: &str,
) -> (Option<String>, bool, String) {
    match TranscriptsRepository::get_transcript_segments_with_speakers(pool, meeting_id).await {
        Ok(segments) => {
            let names_hash =
                speaker_names_fingerprint(segments.iter().filter_map(|(name, _)| name.as_deref()));
            let (attributed, any_speaker) =
                crate::summary::processor::build_speaker_attributed_transcript(&segments);
            (any_speaker.then_some(attributed), any_speaker, names_hash)
        }
        Err(e) => {
            warn!(
                "Failed to load speaker-attributed transcript for meeting_id={meeting_id} \
                 ({e}); using transcript as provided"
            );
            (None, false, speaker_names_fingerprint(std::iter::empty()))
        }
    }
}

/// Pure trigger-decision logic (unit-tested; see also the integration test
/// `tests/summary_speaker_refresh.rs` for the DB round-trip).
pub fn decide_speaker_refresh(
    speakers_assigned: usize,
    snapshot: Option<&SummaryRefreshSnapshot>,
) -> SpeakerRefreshDecision {
    if speakers_assigned == 0 {
        return SpeakerRefreshDecision::SkipNoSpeakersAssigned;
    }
    let Some(snapshot) = snapshot else {
        return SpeakerRefreshDecision::SkipNoSummary;
    };
    match snapshot.status.to_ascii_lowercase().as_str() {
        "pending" | "processing" => return SpeakerRefreshDecision::SkipSummaryInFlight,
        "completed" => {}
        _ => return SpeakerRefreshDecision::SkipSummaryNotCompleted,
    }
    if snapshot.speaker_attributed {
        return SpeakerRefreshDecision::SkipAlreadyAttributed;
    }
    // Pristine guard: both the generation-time fingerprint and the current markdown
    // must exist and match. Anything unverifiable counts as edited.
    let (Some(expected), Some(markdown)) = (
        snapshot.generated_markdown_hash.as_deref(),
        snapshot.current_markdown.as_deref(),
    ) else {
        return SpeakerRefreshDecision::SkipUserEdited;
    };
    if stable_text_fingerprint(markdown) != expected {
        return SpeakerRefreshDecision::SkipUserEdited;
    }
    SpeakerRefreshDecision::Refresh
}

/// Pure trigger-decision for the post-NAMING refresh (specs/0044 WS3): the user
/// renamed / assigned / merged a speaker, so the resolved name set may differ from
/// what the stored summary used. Unlike [`decide_speaker_refresh`] this does NOT
/// gate on `speaker_attributed` — a summary refreshed with "Speaker 1/Speaker 2"
/// placeholders is attributed but still stale once real names land. The pristine
/// markdown guard is identical (user edits are never clobbered), and an unchanged
/// name set is the no-op/loop guard.
pub fn decide_name_refresh(
    current_names_hash: &str,
    snapshot: Option<&SummaryRefreshSnapshot>,
) -> SpeakerRefreshDecision {
    let Some(snapshot) = snapshot else {
        return SpeakerRefreshDecision::SkipNoSummary;
    };
    match snapshot.status.to_ascii_lowercase().as_str() {
        "pending" | "processing" => return SpeakerRefreshDecision::SkipSummaryInFlight,
        "completed" => {}
        _ => return SpeakerRefreshDecision::SkipSummaryNotCompleted,
    }
    let (Some(expected), Some(markdown)) = (
        snapshot.generated_markdown_hash.as_deref(),
        snapshot.current_markdown.as_deref(),
    ) else {
        return SpeakerRefreshDecision::SkipUserEdited;
    };
    if stable_text_fingerprint(markdown) != expected {
        return SpeakerRefreshDecision::SkipUserEdited;
    }
    // A pre-0044 row has no stored name set: we can't prove "unchanged", and the
    // user just performed a naming action — refresh (pristine guard already held).
    if snapshot.speaker_names_hash.as_deref() == Some(current_names_hash) {
        return SpeakerRefreshDecision::SkipNamesUnchanged;
    }
    SpeakerRefreshDecision::Refresh
}

/// Loads the decision inputs for a meeting's summary process row (`None` when the
/// meeting has no summary process at all).
pub async fn load_snapshot(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<Option<SummaryRefreshSnapshot>> {
    let Some(process) = SummaryProcessesRepository::get_summary_data(pool, meeting_id)
        .await
        .with_context(|| format!("load summary process for meeting {meeting_id}"))?
    else {
        return Ok(None);
    };

    let current_markdown = process.result.as_deref().and_then(|raw| {
        serde_json::from_str::<serde_json::Value>(raw)
            .ok()?
            .get("markdown")?
            .as_str()
            .map(str::to_string)
    });

    Ok(Some(SummaryRefreshSnapshot {
        status: process.status,
        speaker_attributed: process.speaker_attributed != 0,
        generated_markdown_hash: process.generated_markdown_hash,
        current_markdown,
        speaker_names_hash: process.speaker_names_hash,
    }))
}

/// Entry point, called by the offline diarization pipeline right after it persisted
/// speaker assignments and BEFORE it emits `diarization-complete` — so when the event
/// (carrying `summaryRefreshing: true`) reaches the frontend, the summary process row
/// is already PENDING and polling can't race a stale 'completed'.
///
/// `speakers_assigned` is the pass's persisted distinct-speaker count.
pub async fn refresh_summary_after_diarization<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    speakers_assigned: usize,
) -> Result<SpeakerRefreshOutcome> {
    let snapshot = load_snapshot(pool, meeting_id).await?;
    let decision = decide_speaker_refresh(speakers_assigned, snapshot.as_ref());
    info!(
        "post-diarization summary refresh for meeting {meeting_id}: {decision:?} \
         ({speakers_assigned} speakers assigned)"
    );
    match decision {
        SpeakerRefreshDecision::Refresh => {
            start_refresh(app, pool, meeting_id).await?;
            Ok(SpeakerRefreshOutcome::Started)
        }
        SpeakerRefreshDecision::SkipSummaryInFlight => {
            // The in-flight run may have read the transcript before speakers were
            // persisted. Wait for it to settle on a background task, then re-decide
            // once — bounded so a wedged run can't leak the task forever.
            let app = app.clone();
            let pool = pool.clone();
            let meeting_id = meeting_id.to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = wait_then_refresh(&app, &pool, &meeting_id, speakers_assigned).await
                {
                    warn!(
                        "post-diarization summary refresh (deferred) failed for meeting \
                         {meeting_id}: {e:#}"
                    );
                }
            });
            Ok(SpeakerRefreshOutcome::DeferredBehindInFlightRun)
        }
        skip => Ok(SpeakerRefreshOutcome::Skipped(skip)),
    }
}

/// Debounce window for the post-naming trigger: the owner typically names several
/// speakers in a burst (chip, chip, rename…) — one regeneration at the end, not one
/// per click. Reset-on-call: each new naming event restarts the clock.
const NAME_REFRESH_DEBOUNCE: Duration = Duration::from_secs(20);

/// Per-meeting debounce generations. A scheduled task only fires if its generation
/// is still the latest when the window elapses; newer naming events supersede it.
/// (Counter map, not task aborts — simpler and safe across runtimes.)
static NAME_REFRESH_GENERATIONS: Lazy<Mutex<HashMap<String, u64>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// specs/0044 WS3 — entry point for the four speaker-naming mutations
/// (`api_rename_speaker`, `api_assign_speaker_to_person`,
/// `api_assign_speaker_to_attendee`, `api_merge_speakers`; the suggestion-chip
/// accept routes through these). Fire-and-forget: schedules a debounced check
/// that regenerates a pristine, completed summary iff the resolved name set
/// actually changed since it was generated. Never blocks or fails the mutation.
pub fn schedule_name_refresh<R: Runtime>(app: &AppHandle<R>, pool: SqlitePool, meeting_id: &str) {
    let my_generation = {
        let mut map = NAME_REFRESH_GENERATIONS
            .lock()
            .expect("name-refresh debounce map poisoned");
        let entry = map.entry(meeting_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    };
    let app = app.clone();
    let meeting_id = meeting_id.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(NAME_REFRESH_DEBOUNCE).await;
        let latest = NAME_REFRESH_GENERATIONS
            .lock()
            .expect("name-refresh debounce map poisoned")
            .get(&meeting_id)
            .copied()
            .unwrap_or(0);
        if latest != my_generation {
            return; // superseded by a newer naming event — its task will handle it
        }
        if let Err(e) = run_name_refresh(&app, &pool, &meeting_id).await {
            warn!("post-naming summary refresh failed for meeting {meeting_id}: {e:#}");
        }
    });
}

/// The post-debounce body of the naming trigger: recompute the current name set,
/// decide, and act. Public-in-crate shape mirrors
/// [`refresh_summary_after_diarization`] so tests can drive it without the timer.
pub async fn run_name_refresh<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<SpeakerRefreshOutcome> {
    let current = current_speaker_names_hash(pool, meeting_id).await?;
    let snapshot = load_snapshot(pool, meeting_id).await?;
    let decision = decide_name_refresh(&current, snapshot.as_ref());
    info!("post-naming summary refresh for meeting {meeting_id}: {decision:?}");
    match decision {
        SpeakerRefreshDecision::Refresh => {
            start_refresh(app, pool, meeting_id).await?;
            Ok(SpeakerRefreshOutcome::Started)
        }
        SpeakerRefreshDecision::SkipSummaryInFlight => {
            // The in-flight run may or may not have read the renamed speakers —
            // wait for it to settle, then re-decide once (its persisted names hash
            // tells us; an unchanged-vs-current hash decides Skip, no double run).
            let app = app.clone();
            let pool = pool.clone();
            let meeting_id = meeting_id.to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = wait_then_name_refresh(&app, &pool, &meeting_id).await {
                    warn!(
                        "post-naming summary refresh (deferred) failed for meeting \
                         {meeting_id}: {e:#}"
                    );
                }
            });
            Ok(SpeakerRefreshOutcome::DeferredBehindInFlightRun)
        }
        skip => Ok(SpeakerRefreshOutcome::Skipped(skip)),
    }
}

/// Waits (bounded) for an in-flight summary run to settle, then re-runs the NAMING
/// decision once — recomputing the current name hash each poll, since the settled
/// run persists the hash it actually used.
async fn wait_then_name_refresh<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<()> {
    let started = Instant::now();
    loop {
        tokio::time::sleep(IN_FLIGHT_POLL_INTERVAL).await;
        let current = current_speaker_names_hash(pool, meeting_id).await?;
        let snapshot = load_snapshot(pool, meeting_id).await?;
        match decide_name_refresh(&current, snapshot.as_ref()) {
            SpeakerRefreshDecision::Refresh => {
                info!(
                    "post-naming summary refresh for meeting {meeting_id}: in-flight run \
                     settled with a stale name set — regenerating"
                );
                return start_refresh(app, pool, meeting_id).await;
            }
            SpeakerRefreshDecision::SkipSummaryInFlight => {
                if started.elapsed() >= IN_FLIGHT_MAX_WAIT {
                    warn!(
                        "post-naming summary refresh for meeting {meeting_id}: gave up \
                         waiting for the in-flight summary run after {}s",
                        IN_FLIGHT_MAX_WAIT.as_secs()
                    );
                    return Ok(());
                }
            }
            skip => {
                info!(
                    "post-naming summary refresh for meeting {meeting_id}: in-flight run \
                     settled, no refresh needed ({skip:?})"
                );
                return Ok(());
            }
        }
    }
}

/// Waits (bounded) for an in-flight summary run to reach a terminal state, then
/// re-runs the decision once and refreshes if it says so.
async fn wait_then_refresh<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    speakers_assigned: usize,
) -> Result<()> {
    let started = Instant::now();
    loop {
        tokio::time::sleep(IN_FLIGHT_POLL_INTERVAL).await;
        let snapshot = load_snapshot(pool, meeting_id).await?;
        match decide_speaker_refresh(speakers_assigned, snapshot.as_ref()) {
            SpeakerRefreshDecision::Refresh => {
                info!(
                    "post-diarization summary refresh for meeting {meeting_id}: in-flight \
                     run settled speakerless — regenerating with speaker names"
                );
                return start_refresh(app, pool, meeting_id).await;
            }
            SpeakerRefreshDecision::SkipSummaryInFlight => {
                if started.elapsed() >= IN_FLIGHT_MAX_WAIT {
                    warn!(
                        "post-diarization summary refresh for meeting {meeting_id}: gave up \
                         waiting for the in-flight summary run after {}s",
                        IN_FLIGHT_MAX_WAIT.as_secs()
                    );
                    return Ok(());
                }
            }
            skip => {
                info!(
                    "post-diarization summary refresh for meeting {meeting_id}: in-flight \
                     run settled, no refresh needed ({skip:?})"
                );
                return Ok(());
            }
        }
    }
}

/// Kicks off the regeneration through the exact path auto-summary / one-click
/// summarize use (persisted template, configured provider, shared background service).
///
/// Both trigger paths funnel here — the immediate one (`refresh_summary_after_diarization`
/// decides `Refresh`) and the deferred one (`wait_then_refresh` re-decides after an
/// in-flight run settles) — so this is where [`EVENT_SUMMARY_REFRESH_STARTED`] is emitted:
/// after the process row is reset to PENDING (inside the start call), never on failure.
async fn start_refresh<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<()> {
    crate::summary::commands::start_summary_generation_for_meeting(
        app.clone(),
        pool.clone(),
        meeting_id.to_string(),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))
    .with_context(|| {
        format!("start speaker-attributed summary regeneration for meeting {meeting_id}")
    })?;
    // Best-effort: the regeneration is already running; a failed emit only costs the
    // open page its live "Updating with speaker names…" strip.
    if let Err(e) = app.emit(
        EVENT_SUMMARY_REFRESH_STARTED,
        summary_refresh_started_payload(meeting_id),
    ) {
        warn!("could not emit {EVENT_SUMMARY_REFRESH_STARTED} for meeting {meeting_id}: {e}");
    }
    Ok(())
}

/// Payload for [`EVENT_SUMMARY_REFRESH_STARTED`]. Kept as a pure function so the shape
/// is unit-testable (the emit itself needs an `AppHandle`); `meeting_id` is snake_case
/// to match the `diarization-complete` payload the same listeners already parse.
fn summary_refresh_started_payload(meeting_id: &str) -> serde_json::Value {
    serde_json::json!({ "meeting_id": meeting_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        status: &str,
        speaker_attributed: bool,
        generated_markdown_hash: Option<String>,
        current_markdown: Option<&str>,
    ) -> SummaryRefreshSnapshot {
        SummaryRefreshSnapshot {
            status: status.to_string(),
            speaker_attributed,
            generated_markdown_hash,
            current_markdown: current_markdown.map(str::to_string),
            speaker_names_hash: None,
        }
    }

    fn pristine_speakerless(markdown: &str) -> SummaryRefreshSnapshot {
        snapshot(
            "completed",
            false,
            Some(stable_text_fingerprint(markdown)),
            Some(markdown),
        )
    }

    #[test]
    fn speakerless_pristine_completed_summary_refreshes() {
        let s = pristine_speakerless("## Decisions\nShip it");
        assert_eq!(
            decide_speaker_refresh(3, Some(&s)),
            SpeakerRefreshDecision::Refresh
        );
    }

    #[test]
    fn already_attributed_summary_never_rerun() {
        // The loop guard: a refreshed summary persists speaker_attributed = 1, so a
        // second diarization pass (or a manual re-run) must decide Skip.
        let md = "## Decisions\nShip it";
        let s = snapshot(
            "completed",
            true,
            Some(stable_text_fingerprint(md)),
            Some(md),
        );
        assert_eq!(
            decide_speaker_refresh(3, Some(&s)),
            SpeakerRefreshDecision::SkipAlreadyAttributed
        );
    }

    #[test]
    fn user_edited_summary_is_never_clobbered() {
        // Hash was taken at generation; the user then edited the markdown.
        let s = snapshot(
            "completed",
            false,
            Some(stable_text_fingerprint("## Decisions\nShip it")),
            Some("## Decisions\nShip it\n\nMy own notes appended"),
        );
        assert_eq!(
            decide_speaker_refresh(3, Some(&s)),
            SpeakerRefreshDecision::SkipUserEdited
        );
    }

    #[test]
    fn legacy_row_without_fingerprint_counts_as_edited() {
        // Rows completed before the 0041 migration have no hash — we can't prove
        // pristine, so never auto-regenerate them.
        let s = snapshot("completed", false, None, Some("## Decisions\nShip it"));
        assert_eq!(
            decide_speaker_refresh(3, Some(&s)),
            SpeakerRefreshDecision::SkipUserEdited
        );
    }

    #[test]
    fn unparseable_or_missing_markdown_counts_as_edited() {
        let s = snapshot(
            "completed",
            false,
            Some(stable_text_fingerprint("anything")),
            None,
        );
        assert_eq!(
            decide_speaker_refresh(3, Some(&s)),
            SpeakerRefreshDecision::SkipUserEdited
        );
    }

    #[test]
    fn no_speakers_assigned_never_refreshes() {
        let s = pristine_speakerless("## Decisions\nShip it");
        assert_eq!(
            decide_speaker_refresh(0, Some(&s)),
            SpeakerRefreshDecision::SkipNoSpeakersAssigned
        );
    }

    #[test]
    fn missing_summary_row_never_refreshes() {
        assert_eq!(
            decide_speaker_refresh(3, None),
            SpeakerRefreshDecision::SkipNoSummary
        );
    }

    #[test]
    fn in_flight_summary_defers() {
        // create_or_reset_process stores 'PENDING' (uppercase) — the decision must be
        // case-insensitive.
        for status in ["PENDING", "processing"] {
            let s = snapshot("completed", false, None, None);
            let s = SummaryRefreshSnapshot {
                status: status.to_string(),
                ..s
            };
            assert_eq!(
                decide_speaker_refresh(3, Some(&s)),
                SpeakerRefreshDecision::SkipSummaryInFlight,
                "status {status}"
            );
        }
    }

    /// The `summary-refresh-started` payload contract: snake_case `meeting_id`, matching
    /// the `diarization-complete` payload the frontend listeners already parse. (The emit
    /// itself needs an `AppHandle`, so `start_refresh`'s event is exercised end-to-end by
    /// the manual smoke path; the payload shape is pinned here. Both the immediate
    /// `Refresh` decision and the deferred `wait_then_refresh` settle path route through
    /// `start_refresh`, so this one emit covers both.)
    #[test]
    fn refresh_started_payload_uses_snake_case_meeting_id() {
        let payload = summary_refresh_started_payload("meeting-42");
        assert_eq!(payload, serde_json::json!({ "meeting_id": "meeting-42" }));
    }

    // --- specs/0044 WS3: post-naming refresh decision ---

    fn pristine_with_names(
        markdown: &str,
        names: &[&str],
        attributed: bool,
    ) -> SummaryRefreshSnapshot {
        SummaryRefreshSnapshot {
            status: "completed".to_string(),
            speaker_attributed: attributed,
            generated_markdown_hash: Some(stable_text_fingerprint(markdown)),
            current_markdown: Some(markdown.to_string()),
            speaker_names_hash: Some(speaker_names_fingerprint(names.iter().copied())),
        }
    }

    #[test]
    fn naming_change_on_pristine_summary_refreshes_even_when_attributed() {
        // THE relaxation over decide_speaker_refresh: a summary refreshed with
        // "Speaker 1/2" placeholders is speaker_attributed=1, but naming Speaker 2
        // → "Priya" changes the set, so it must refresh anyway.
        let md = "## Decisions\nShip it";
        let s = pristine_with_names(md, &["You", "Speaker 1", "Speaker 2"], true);
        let current = speaker_names_fingerprint(["You", "Speaker 1", "Priya"]);
        assert_eq!(
            decide_name_refresh(&current, Some(&s)),
            SpeakerRefreshDecision::Refresh
        );
    }

    #[test]
    fn unchanged_name_set_is_the_noop_and_loop_guard() {
        let md = "## Decisions\nShip it";
        let s = pristine_with_names(md, &["You", "Priya"], true);
        let current = speaker_names_fingerprint(["Priya", "You"]); // order-insensitive
        assert_eq!(
            decide_name_refresh(&current, Some(&s)),
            SpeakerRefreshDecision::SkipNamesUnchanged
        );
    }

    #[test]
    fn naming_never_clobbers_an_edited_summary() {
        let s = SummaryRefreshSnapshot {
            status: "completed".to_string(),
            speaker_attributed: true,
            generated_markdown_hash: Some(stable_text_fingerprint("## Decisions\nShip it")),
            current_markdown: Some("## Decisions\nShip it\n\nmy edits".to_string()),
            speaker_names_hash: Some(speaker_names_fingerprint(["Speaker 1"])),
        };
        assert_eq!(
            decide_name_refresh(&speaker_names_fingerprint(["Priya"]), Some(&s)),
            SpeakerRefreshDecision::SkipUserEdited
        );
    }

    #[test]
    fn legacy_row_without_names_hash_refreshes_when_pristine() {
        // Pre-0044 rows can't prove "unchanged"; the user just named someone, so a
        // pristine summary refreshes once (and persists the hash going forward).
        let md = "## Decisions\nShip it";
        let s = snapshot(
            "completed",
            true,
            Some(stable_text_fingerprint(md)),
            Some(md),
        );
        assert_eq!(
            decide_name_refresh(&speaker_names_fingerprint(["Priya"]), Some(&s)),
            SpeakerRefreshDecision::Refresh
        );
    }

    #[test]
    fn naming_with_no_summary_or_inflight_or_failed_skips() {
        let current = speaker_names_fingerprint(["Priya"]);
        assert_eq!(
            decide_name_refresh(&current, None),
            SpeakerRefreshDecision::SkipNoSummary
        );
        let inflight = snapshot("PENDING", false, None, None);
        assert_eq!(
            decide_name_refresh(&current, Some(&inflight)),
            SpeakerRefreshDecision::SkipSummaryInFlight
        );
        let failed = snapshot("failed", false, None, None);
        assert_eq!(
            decide_name_refresh(&current, Some(&failed)),
            SpeakerRefreshDecision::SkipSummaryNotCompleted
        );
    }

    #[test]
    fn names_fingerprint_ignores_order_dupes_blanks_and_whitespace() {
        let a = speaker_names_fingerprint(["Priya", "You", "  Priya  ", "", "  "]);
        let b = speaker_names_fingerprint(["You", "Priya"]);
        assert_eq!(a, b);
        let c = speaker_names_fingerprint(["You", "Priya", "Sam"]);
        assert_ne!(a, c);
        // Empty set has a stable, distinct fingerprint.
        assert_eq!(
            speaker_names_fingerprint(std::iter::empty()),
            speaker_names_fingerprint(["", "  "])
        );
    }

    #[test]
    fn failed_or_cancelled_summary_skips() {
        for status in ["failed", "cancelled"] {
            let md = "## Decisions\nShip it";
            let s = snapshot(status, false, Some(stable_text_fingerprint(md)), Some(md));
            assert_eq!(
                decide_speaker_refresh(3, Some(&s)),
                SpeakerRefreshDecision::SkipSummaryNotCompleted,
                "status {status}"
            );
        }
    }
}
