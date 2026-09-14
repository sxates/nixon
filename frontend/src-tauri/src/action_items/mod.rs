//! Action items v1 (specs/0034): structured, regeneration-safe action items extracted
//! from meeting summaries, plus the manual/task-hub surface.
//!
//! Layout mirrors `people/`:
//! - [`extractor`] — the LLM candidate call (prompt, defensive parse, single retry) +
//!   owner resolution;
//! - [`diff`] — the pure protected/pristine diff engine (the safety property);
//! - [`commands`] — the 7 Tauri commands;
//! - [`run_extraction`] — the shared orchestration used by BOTH the background trigger
//!   (`summary/service.rs`, after `update_process_completed` succeeds) and the manual
//!   `api_extract_action_items` command.
//!
//! DB I/O lives in `database::repositories::action_item::ActionItemsRepository`.

pub mod commands;
pub mod diff;
pub mod extractor;
pub mod grammar;
mod reply_parse;
pub mod skip_gate;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Context;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tracing::{info, warn};

use crate::database::repositories::action_item::{ActionItem, ActionItemsRepository};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;
use crate::llm_activity::{LlmActivityState, Origin, TaskKind};
use crate::summary::provider_config::ProviderConfig;

/// Rust → frontend event emitted after an extraction transaction commits. Payload:
/// `{ "meeting_id": string }`. The meeting page and the task hub refresh on it
/// (user-initiated commands just use their return values).
pub const ACTION_ITEMS_UPDATED_EVENT: &str = "action-items-updated";

/// Meeting ids with an extraction currently in flight. Extraction runs OVERLAP by
/// design — the background post-summary spawn and a user's "Scan again" are the app's
/// natural state — and two concurrent extract+diff passes over the same meeting would
/// each diff against the same snapshot and double-insert everything. One global set
/// (not per-AppState) because the pool is global too.
static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn in_flight() -> &'static Mutex<HashSet<String>> {
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// RAII in-flight marker for one meeting's extraction. Dropping it — on success, early
/// return, or error unwind — releases the slot, so a failed run never wedges a meeting.
struct InFlightGuard {
    meeting_id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        in_flight()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.meeting_id);
    }
}

/// Try to claim the per-meeting extraction slot. `None` = another run for this meeting
/// is already in flight and the caller must skip (treat as a no-op, not an error).
fn try_claim_extraction(meeting_id: &str) -> Option<InFlightGuard> {
    let mut set = in_flight()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    set.insert(meeting_id.to_string()).then(|| InFlightGuard {
        meeting_id: meeting_id.to_string(),
    })
}

/// Descriptions for the extraction prompt's ALREADY TRACKED section: the PROTECTED rows
/// only — manual, user-edited, completed, and dismissed (a rephrased dismissed task must
/// not resurrect; the extractor caps the list at `extractor::ALREADY_TRACKED_CAP`).
///
/// PRISTINE rows are deliberately excluded. Listing them would tell the model to omit
/// their tasks from the reply, and the diff deletes unmatched pristine rows ("the new
/// summary no longer supports them") — so every regeneration would silently wipe the
/// user's untouched open items. Pristine rows don't need prompt dedup anyway: a weak
/// match there nets out as delete-old + insert-new (one row, no duplicate); only
/// protected rows — which survive unmatched forever — can double up, which is exactly
/// what the 2026-07 template-switch incident produced.
fn already_tracked_descriptions(existing: &[ActionItem]) -> Vec<&str> {
    existing
        .iter()
        .filter(|item| diff::is_protected(item))
        .map(|item| item.description.as_str())
        .collect()
}

/// The zero-candidate safety valve (2026-07 incident): an extraction run that returns
/// ZERO candidates while the diff wants to delete pristine rows is treated as SUSPECT,
/// not as truth — an over-suppressed prompt (the ALREADY TRACKED section) or a flaky
/// model must never wipe the user's open items. When this fires (logging a warning), the
/// caller applies NOTHING and does NOT write the ledger fingerprint, so a later run
/// retries against the same summary.
///
/// Deliberate edge: a summary that genuinely contains no action items AND no pristine
/// rows to protect (no rows at all, or protected-only rows — either way
/// `plan.delete_ids` is empty) does NOT trip the valve. There is nothing to delete, and
/// writing the ledger is harmless — it preserves idempotence for genuinely empty
/// summaries.
///
/// `pub` (not `pub(crate)`) so the `action_items_lifecycle` integration test exercises
/// the REAL guard rather than a mirror of it.
pub fn skip_zero_candidate_deletes(
    meeting_id: &str,
    candidate_count: u32,
    plan: &diff::ExtractionDiff,
) -> bool {
    if candidate_count == 0 && !plan.delete_ids.is_empty() {
        warn!(
            "Action-item extraction for meeting_id {}: extractor returned no candidates; \
             skipping {} pristine delete(s) as a safety valve (nothing applied, ledger not \
             written — a later run will retry this summary)",
            meeting_id,
            plan.delete_ids.len()
        );
        return true;
    }
    false
}

/// The whole extraction pass for one meeting: in-flight claim → ledger skip-check →
/// roster + owner identity + existing rows (protected descriptions feed the prompt's
/// ALREADY TRACKED dedup) → LLM candidates → owner resolution → protected/pristine diff
/// → one atomic apply + ledger upsert → [`ACTION_ITEMS_UPDATED_EVENT`].
///
/// `provider_config` must be the EXACT values the summary run used (specs/0034, decided
/// 2026-07-03: no separate extraction provider setting).
///
/// Returns the number of rows this run inserted or updated. `0` means the ledger stayed
/// as it was OR the run wrote nothing new: a fingerprint-match no-op, another run already
/// in flight for this meeting, the meeting deleted mid-run (apply aborted), every
/// candidate absorbed by protected rows, or a suspect zero-candidate result skipped by
/// [`skip_zero_candidate_deletes`] (nothing applied, ledger untouched so a later run
/// retries). It is NOT the extractor's candidate count (the
/// ledger's `item_count` still records that). Errors are `anyhow` with user-actionable
/// context — the background trigger logs them, the manual command surfaces them.
pub async fn run_extraction<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    summary_markdown: &str,
    user_notes: Option<&str>,
    provider_config: &ProviderConfig,
) -> anyhow::Result<u32> {
    // specs/0052: register as background work so the sidebar can show it. Extraction is
    // fire-and-forget after a summary completes and previously had no UI signal at all.
    let registry = app
        .try_state::<LlmActivityState>()
        .map(|state| Arc::clone(&state.0));
    let task = match registry {
        Some(registry) => {
            let title = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
                Ok(Some(meta)) => format!("Extracting action items — {}", meta.title),
                _ => "Extracting action items".to_string(),
            };
            Some(registry.start_for(
                TaskKind::ActionItems,
                Origin::Background,
                title,
                Some(meeting_id.to_string()),
            ))
        }
        None => None,
    };

    let result = run_extraction_inner(
        app,
        pool,
        meeting_id,
        summary_markdown,
        user_notes,
        provider_config,
    )
    .await;

    if let Some(t) = task {
        t.finish(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
    }
    result
}

/// Record a skipped extraction in the LLM activity registry (specs/0052), so a
/// skip is visible rather than silent. Writes NO ledger row — a later manual
/// "Scan again" must still run for real.
pub async fn record_extraction_skipped<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    reason: &str,
) {
    let Some(state) = app.try_state::<LlmActivityState>() else {
        return;
    };
    let title = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
        Ok(Some(meta)) => format!("Extracting action items — {}", meta.title),
        _ => "Extracting action items".to_string(),
    };
    let task = Arc::clone(&state.0).start_for(
        TaskKind::ActionItems,
        Origin::Background,
        title,
        Some(meeting_id.to_string()),
    );
    task.finish_skipped(reason);
}

/// specs/0053 W3: the skip gate for the BACKGROUND spawn only, reading the
/// stored Auto outline and recording the skip (never silently — specs/0052)
/// when it fires. Returns `true` when the caller must not run extraction.
async fn maybe_skip_background_extraction<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    template_id: &str,
    user_notes: Option<&str>,
) -> bool {
    let stored = crate::database::repositories::summary_outline::SummaryOutlineRepository::get(
        pool, meeting_id,
    )
    .await
    .unwrap_or(None);
    let has_user_notes = user_notes.is_some_and(|n| !n.trim().is_empty());
    if !skip_gate::should_skip_background_extraction(template_id, stored.as_ref(), has_user_notes) {
        return false;
    }
    info!(
        "Action-item extraction skipped for meeting_id {}: the outline found no commitments",
        meeting_id
    );
    record_extraction_skipped(
        app,
        pool,
        meeting_id,
        "No commitments found in this meeting",
    )
    .await;
    true
}

/// The whole background post-summary sequence (specs/0034 trigger + specs/0053
/// W3 skip gate): skip when the Auto outline found nothing to extract,
/// otherwise run extraction and log — never propagate — any failure, so the
/// (already persisted) summary is never affected by an extraction problem.
pub async fn run_background_extraction<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    template_id: &str,
    summary_markdown: &str,
    user_notes: Option<&str>,
    provider_config: &ProviderConfig,
) {
    if maybe_skip_background_extraction(app, pool, meeting_id, template_id, user_notes).await {
        return;
    }
    if let Err(e) = run_extraction(
        app,
        pool,
        meeting_id,
        summary_markdown,
        user_notes,
        provider_config,
    )
    .await
    {
        // specs/0056 W7: name the backend — a multi-provider user cannot otherwise tell from
        // this line which model produced the failure (0054 promised this, then shipped without).
        warn!(
            "Action-item extraction failed for meeting_id {} (provider {:?}, model {}; summary unaffected): {:#}",
            meeting_id, provider_config.provider, provider_config.model_name, e
        );
    }
}

async fn run_extraction_inner<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    summary_markdown: &str,
    user_notes: Option<&str>,
    provider_config: &ProviderConfig,
) -> anyhow::Result<u32> {
    // Serialize per meeting: the post-summary background spawn and a manual "Scan again"
    // can overlap; the second run would diff against the first run's pre-commit snapshot
    // and double-insert every item. Skipping is safe — the in-flight run is extracting
    // from the same stored summary.
    let Some(_in_flight_guard) = try_claim_extraction(meeting_id) else {
        info!(
            "Action-item extraction skipped for meeting_id {}: another extraction is already in flight",
            meeting_id
        );
        return Ok(0);
    };

    // Idempotence (acceptance #4): a regeneration that produced identical summary+notes
    // is a no-op — zero row churn, `updated_at` untouched.
    let fingerprint = diff::extraction_fingerprint(summary_markdown, user_notes);
    let ledger = ActionItemsRepository::get_extraction(pool, meeting_id)
        .await
        .context("Failed to read the action-item extraction ledger")?;
    if let Some(ledger) = ledger {
        if ledger.summary_fingerprint == fingerprint {
            info!(
                "Action-item extraction skipped for meeting_id {}: summary unchanged (ledger fingerprint match)",
                meeting_id
            );
            return Ok(0);
        }
    }

    let roster = MeetingParticipantsRepository::list(pool, meeting_id)
        .await
        .context("Failed to load the meeting's participant roster")?;
    // The owner's identity feeds BOTH prompt (the "me" option carries it) and resolution
    // ("You"/owner-name/owner-email → assignee_is_self) — the roster excludes self.
    let owner = extractor::OwnerIdentity::load(pool).await?;

    // Load the existing rows BEFORE the LLM call: the protected ones feed the prompt's
    // ALREADY TRACKED section (prompt-level dedup) and the same snapshot feeds the diff
    // afterwards — one query, one consistent view (the in-flight guard above serializes
    // concurrent runs, so nothing mutates it mid-pass).
    let existing = ActionItemsRepository::get_for_meeting(pool, meeting_id)
        .await
        .context("Failed to load existing action items")?;
    let already_tracked = already_tracked_descriptions(&existing);

    let client = reqwest::Client::new();
    let app_data_dir = app.path().app_data_dir().ok();
    let candidates = extractor::extract_candidates(
        &client,
        provider_config,
        app_data_dir.as_ref(),
        summary_markdown,
        user_notes,
        &already_tracked,
        &roster,
        &owner,
    )
    .await
    .context("Action-item extraction failed")?;
    let candidate_count = candidates.len() as u32;

    let resolved = extractor::resolve_candidates(candidates, &roster, &owner);
    let plan = diff::compute_diff(&existing, &resolved);

    info!(
        "Action-item extraction for meeting_id {}: {} candidate(s) → {} insert(s), {} update(s), {} delete(s)",
        meeting_id,
        candidate_count,
        plan.inserts.len(),
        plan.updates.len(),
        plan.delete_ids.len()
    );

    // Zero-candidate safety valve: apply nothing and leave the ledger alone (see the
    // guard's doc — the 2026-07 incident's "0 candidate(s) → ... 1 delete(s)" run).
    if skip_zero_candidate_deletes(meeting_id, candidate_count, &plan) {
        return Ok(0);
    }

    let rows_written = match ActionItemsRepository::replace_extracted(
        pool,
        meeting_id,
        &plan,
        &fingerprint,
        &provider_config.model_provider,
        &provider_config.model_name,
        candidate_count,
    )
    .await
    .context("Failed to save extracted action items")?
    {
        Some(rows_written) => rows_written,
        None => {
            // The meeting was deleted while the LLM call was in flight; the apply rolled
            // back rather than resurrecting orphaned rows. Not an error — the user asked
            // for the meeting (and its items) to be gone.
            info!(
                "Action-item extraction aborted for meeting_id {}: meeting deleted mid-run (nothing written)",
                meeting_id
            );
            return Ok(0);
        }
    };

    let _ = app.emit(
        ACTION_ITEMS_UPDATED_EVENT,
        serde_json::json!({ "meeting_id": meeting_id }),
    );

    Ok(rows_written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        id: &str,
        description: &str,
        status: &str,
        source: &str,
        user_edited: bool,
    ) -> ActionItem {
        ActionItem {
            id: id.to_string(),
            meeting_id: Some("m1".to_string()),
            description: description.to_string(),
            assignee_person_id: None,
            assignee_is_self: false,
            assignee_raw: None,
            due_hint: None,
            due_date: None,
            status: status.to_string(),
            source: source.to_string(),
            user_edited,
            content_key: diff::content_key(description),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            updated_at: "2026-07-01T00:00:00Z".to_string(),
            completed_at: None,
            sort_order: None,
        }
    }

    /// The ALREADY TRACKED prompt list carries every PROTECTED row — completed,
    /// dismissed, user-edited, and manual — and deliberately excludes pristine rows
    /// (listing those would make the model omit them and the diff delete them; see
    /// `already_tracked_descriptions`).
    #[test]
    fn already_tracked_lists_protected_rows_of_every_kind_but_not_pristine() {
        let existing = vec![
            item("ai-1", "Set up the room", "completed", "extracted", false),
            item(
                "ai-2",
                "Follow up with legal",
                "dismissed",
                "extracted",
                false,
            ),
            item(
                "ai-3",
                "Send the presentation deck to participants",
                "open",
                "extracted",
                true,
            ),
            item("ai-4", "Water the office plants", "open", "manual", false),
            item("ai-5", "Book the retro room", "open", "extracted", false), // pristine
        ];
        let tracked = already_tracked_descriptions(&existing);
        assert_eq!(
            tracked,
            vec![
                "Set up the room",
                "Follow up with legal",
                "Send the presentation deck to participants",
                "Water the office plants",
            ],
            "all protected statuses/sources listed; pristine excluded"
        );
    }

    /// Regression (concurrent-run double-insert): the per-meeting in-flight guard admits
    /// one claimant, is meeting-scoped, and releases on drop — including mid-scope drops,
    /// which is what the RAII guard does on `run_extraction`'s early returns and `?`
    /// error paths.
    #[test]
    fn in_flight_guard_blocks_second_claim_and_releases_on_drop() {
        let guard = try_claim_extraction("m-inflight-a").expect("first claim succeeds");
        assert!(
            try_claim_extraction("m-inflight-a").is_none(),
            "second concurrent claim for the same meeting must be refused"
        );
        // Other meetings are unaffected (the guard is per-meeting, not global).
        assert!(
            try_claim_extraction("m-inflight-b").is_some(),
            "a different meeting claims independently"
        );

        drop(guard);
        assert!(
            try_claim_extraction("m-inflight-a").is_some(),
            "dropping the guard releases the slot"
        );
    }

    fn plan_with_deletes(n: usize) -> diff::ExtractionDiff {
        diff::ExtractionDiff {
            delete_ids: (0..n).map(|i| format!("ai-{i}")).collect(),
            ..Default::default()
        }
    }

    /// Regression (2026-07 zero-candidate incident): a run whose extractor returned no
    /// candidates must not delete pristine rows — but the valve fires ONLY when there is
    /// something pristine to protect. Zero candidates with an empty delete plan (no rows,
    /// or protected-only rows) proceeds normally, so a genuinely empty summary still
    /// writes its ledger fingerprint (idempotence).
    #[test]
    fn zero_candidate_valve_fires_only_when_pristine_deletes_are_pending() {
        assert!(
            skip_zero_candidate_deletes("m1", 0, &plan_with_deletes(1)),
            "the incident shape: 0 candidates, 1 pending pristine delete"
        );
        assert!(
            !skip_zero_candidate_deletes("m1", 0, &plan_with_deletes(0)),
            "genuinely empty summary with nothing to delete → proceed (ledger written)"
        );
        assert!(
            !skip_zero_candidate_deletes("m1", 2, &plan_with_deletes(1)),
            "a run WITH candidates may delete pristine rows (normal regeneration)"
        );
    }

    /// The valve logs a user-diagnosable warning naming the meeting and the skipped
    /// delete count (the incident was diagnosed from exactly this kind of log line).
    #[test]
    fn zero_candidate_valve_logs_a_warning() {
        use std::io::Write;
        use std::sync::Arc;

        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl Write for Buf {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Buf::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::WARN)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert!(skip_zero_candidate_deletes(
                "m-warn",
                0,
                &plan_with_deletes(3)
            ));
        });

        let logged = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("extractor returned no candidates"),
            "warning names the cause: {logged}"
        );
        assert!(
            logged.contains("skipping 3 pristine delete(s) as a safety valve"),
            "warning names the skipped delete count: {logged}"
        );
        assert!(
            logged.contains("m-warn"),
            "warning names the meeting: {logged}"
        );
    }
}
