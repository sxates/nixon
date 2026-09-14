//! Pre-call-prep IPC (specs/0036): the Prep tab's read + the prep-notes autosave, plus
//! on-demand brief (re)generation with `prep-brief-*` progress events.
//!
//! Briefs are normally warm (the background `prep_jobs` generator). These commands cover the
//! gaps: opening a meeting whose brief the background pass hasn't reached yet, a manual
//! refresh, and minting a scheduled prep row when the user opens an upcoming event. All
//! generation routes through the shared `prep_jobs::generate_brief_for_target`.

use crate::aggregation::engine::{SourceMeeting, Stage};
use crate::aggregation::prep_jobs::generate_brief_for_target;
use crate::database::repositories::action_item::{ActionItem, ActionItemsRepository};
use crate::database::repositories::meeting::{MeetingsRepository, SeriesLinkedMeeting};
use crate::database::repositories::meeting_brief::MeetingBriefsRepository;
use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::state::AppState;
use crate::summary::llm_gate::{with_priority, Priority};
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const MAX_PRIOR: i64 = 2;

// ---------------------------------------------------------------------------
// Generation-run registry (one in-flight brief per meeting)
// ---------------------------------------------------------------------------

/// Active runs keyed by `meeting_id`. The value pairs a unique `run_id` with the run's
/// cancellation token so teardown can verify identity: a cancelled run must NOT remove the
/// entry of the *newer* run that replaced it (otherwise the single-flight guard is defeated
/// and two generations race — code-review A2).
static PREP_RUNS: Lazy<Mutex<HashMap<String, (String, CancellationToken)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Registers a run for `meeting_id` and returns its `(run_id, token)`, or `None` if one is
/// already in flight (caller should not start a second).
fn try_register(meeting_id: &str) -> Option<(String, CancellationToken)> {
    let mut runs = PREP_RUNS.lock().expect("prep run registry poisoned");
    if runs.contains_key(meeting_id) {
        return None;
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    let token = CancellationToken::new();
    runs.insert(meeting_id.to_string(), (run_id.clone(), token.clone()));
    Some((run_id, token))
}

/// Removes the registry entry for `meeting_id` ONLY if it still belongs to `run_id` — so a
/// finishing (possibly cancelled) run never clobbers a newer run that replaced it.
fn finish_run(meeting_id: &str, run_id: &str) {
    let mut runs = PREP_RUNS.lock().expect("prep run registry poisoned");
    if runs.get(meeting_id).map(|(rid, _)| rid.as_str()) == Some(run_id) {
        runs.remove(meeting_id);
    }
}

/// Cancels an in-flight generation for `meeting_id` (used before a forced regenerate).
fn cancel_run(meeting_id: &str) {
    if let Some((_, token)) = PREP_RUNS
        .lock()
        .expect("prep run registry poisoned")
        .remove(meeting_id)
    {
        token.cancel();
    }
}

// ---------------------------------------------------------------------------
// Event payloads (kebab-case names, camelCase payloads — the ask-ai-* convention)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepProgressPayload {
    meeting_id: String,
    stage: &'static str,
    current: usize,
    total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepCompletePayload {
    meeting_id: String,
    status: String,
    markdown: Option<String>,
    sources: Vec<SourceMeeting>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepErrorPayload {
    meeting_id: String,
    message: String,
}

fn stage_str(stage: Stage) -> (&'static str, usize, usize) {
    match stage {
        Stage::Gathering => ("gathering", 0, 0),
        Stage::Mapping { current, total } => ("mapping", current, total),
        Stage::Reducing => ("reducing", 0, 0),
    }
}

/// Spawn on-demand brief generation for a meeting, emitting `prep-brief-*` progress and, on
/// finish, `prep-briefs-updated` (so the Today view / open Prep tab refresh). No-ops if a run
/// is already in flight for the meeting (unless it was just cancelled by a forced regenerate).
fn spawn_generation<R: Runtime>(app: &AppHandle<R>, meeting_id: String, force: bool) {
    let Some((run_id, cancel)) = try_register(&meeting_id) else {
        return; // already generating
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<AppState>() else {
            finish_run(&meeting_id, &run_id);
            return;
        };
        let pool = state.db_manager.pool().clone();

        let progress_app = app.clone();
        let progress_id = meeting_id.clone();
        let on_progress = move |stage: Stage| {
            let (stage, current, total) = stage_str(stage);
            let _ = progress_app.emit(
                "prep-brief-progress",
                PrepProgressPayload {
                    meeting_id: progress_id.clone(),
                    stage,
                    current,
                    total,
                },
            );
        };

        // specs/0056 W2: an explicit "Regenerate" click is interactive; the lazy generation
        // kicked off by merely opening the Prep tab stays background work.
        let priority = if force {
            Priority::Interactive
        } else {
            Priority::Background
        };
        let result = with_priority(
            priority,
            generate_brief_for_target(&app, &pool, &meeting_id, force, &cancel, on_progress),
        )
        .await;

        match result {
            Ok(()) => {
                // Emit the freshly-cached brief (status may be ready | none | pending).
                match MeetingBriefsRepository::get(&pool, &meeting_id).await {
                    Ok(Some(row)) => {
                        let sources = row
                            .sources_json
                            .as_deref()
                            .and_then(|j| serde_json::from_str::<Vec<SourceMeeting>>(j).ok())
                            .unwrap_or_default();
                        let _ = app.emit(
                            "prep-brief-complete",
                            PrepCompletePayload {
                                meeting_id: meeting_id.clone(),
                                status: row.status,
                                markdown: row.brief_markdown,
                                sources,
                            },
                        );
                    }
                    _ => {
                        let _ = app.emit(
                            "prep-brief-complete",
                            PrepCompletePayload {
                                meeting_id: meeting_id.clone(),
                                status: "none".into(),
                                markdown: None,
                                sources: Vec::new(),
                            },
                        );
                    }
                }
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    info!("prep: generation for {} cancelled", meeting_id);
                } else {
                    warn!("prep: generation for {} failed: {:#}", meeting_id, e);
                    let _ = app.emit(
                        "prep-brief-error",
                        PrepErrorPayload {
                            meeting_id: meeting_id.clone(),
                            message: format!("{e:#}"),
                        },
                    );
                }
            }
        }

        finish_run(&meeting_id, &run_id);
        let _ = app.emit("prep-briefs-updated", ());
    });
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// The Prep tab's view model for one meeting.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepView {
    pub meeting_id: String,
    pub origin: String,
    pub title: String,
    /// 'pending' | 'ready' | 'failed' | 'none' | 'absent' (absent = generation just kicked off).
    pub brief_status: String,
    pub brief_markdown: Option<String>,
    pub brief_sources: Vec<SourceMeeting>,
    /// Open action items carried over from the series' prior occurrences, mine-first. The
    /// frontend splits mine (`assigneeIsSelf`) from owed-by-others and resolves people.
    pub open_items: Vec<ActionItem>,
    pub prep_notes_markdown: Option<String>,
    pub prep_notes_json: Option<String>,
    /// Meetings MANUALLY linked into this meeting's series (specs/0041 WS4), newest first,
    /// excluding this meeting. Backs the "Linked meetings" list + unlink affordance.
    pub linked_meetings: Vec<SeriesLinkedMeeting>,
}

/// Read the Prep view for a meeting. If no brief is cached yet, kicks off generation in the
/// background (progress arrives via `prep-brief-*`) and returns `briefStatus: "absent"`.
#[tauri::command]
pub async fn api_get_prep<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<PrepView, String> {
    let pool = state.db_manager.pool();

    let meta = MeetingsRepository::get_meeting_metadata(pool, &meeting_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Meeting not found".to_string())?;

    // EFFECTIVE series key (specs/0041 WS4): calendar-stamped, else the manual
    // meeting_series_links key — so manually associated series behave like calendar ones.
    let series_key = MeetingsRepository::resolve_effective_series_key(pool, &meeting_id)
        .await
        .map_err(|e| format!("{e}"))?;

    // Carryover: open items from the series' prior occurrences (series key ∪ title ∪ links).
    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        series_key.as_deref(),
        &meta.title,
        meta.created_at.0,
        MAX_PRIOR,
    )
    .await
    .map_err(|e| format!("{e}"))?;

    // Manually linked meetings, for the "Linked meetings" list + unlink affordance.
    let linked_meetings = match series_key.as_deref() {
        Some(key) => MeetingsRepository::linked_meetings_for_series(pool, key, &meeting_id)
            .await
            .map_err(|e| format!("{e}"))?,
        None => Vec::new(),
    };
    let open_items = ActionItemsRepository::get_open_for_meetings(pool, &prior)
        .await
        .map_err(|e| format!("{e}"))?;

    // Prep notes.
    let (prep_notes_markdown, prep_notes_json) =
        match MeetingNotesRepository::get_notes(pool, &meeting_id).await {
            Ok(Some(n)) => (n.prep_markdown, n.prep_json),
            _ => (None, None),
        };

    // Brief: from cache, or spawn generation if we've never generated one.
    let brief = MeetingBriefsRepository::get(pool, &meeting_id)
        .await
        .map_err(|e| format!("{e}"))?;
    let (brief_status, brief_markdown, brief_sources) = match brief {
        // A cached 'none' is stale the moment the prior set becomes non-empty (a manual
        // link elsewhere, or a new same-title recording the specs/0041 union now matches):
        // regenerate on open instead of showing the empty state until the next background
        // pass. `generate_brief_for_target` recomputes the prior set itself, so this is safe.
        Some(row) if row.status == "none" && !prior.is_empty() => {
            spawn_generation(&app, meeting_id.clone(), false);
            ("absent".to_string(), None, Vec::new())
        }
        Some(row) => {
            let sources = row
                .sources_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<SourceMeeting>>(j).ok())
                .unwrap_or_default();
            (row.status, row.brief_markdown, sources)
        }
        None => {
            // Only worth generating when there's prior history to brief from.
            if !prior.is_empty() {
                spawn_generation(&app, meeting_id.clone(), false);
                ("absent".to_string(), None, Vec::new())
            } else {
                ("none".to_string(), None, Vec::new())
            }
        }
    };

    Ok(PrepView {
        meeting_id: meta.id,
        origin: meta.origin,
        title: meta.title,
        brief_status,
        brief_markdown,
        brief_sources,
        open_items,
        prep_notes_markdown,
        prep_notes_json,
        linked_meetings,
    })
}

/// Manually link `meeting_id` (a previous recording) into the series of
/// `target_meeting_id` (the meeting whose Prep tab the user is on). Key resolution —
/// target's `calendar_series_key`, else its existing link key, else a minted
/// `manual:{uuid}` stamped on both rows — lives in
/// `MeetingsRepository::link_meeting_to_series`.
///
/// Regenerate-on-link (specs/0041 WS4.4): the prior set for EVERY occurrence of the series
/// just changed, so all of the series' cached briefs are deleted (a stale 'none'/'ready' row
/// would otherwise block `api_get_prep`, which only generates when no row exists) and a
/// fresh generation is kicked off for the target. Progress arrives via `prep-brief-*`;
/// `prep-briefs-updated` fires when it lands.
#[tauri::command]
pub async fn api_link_meeting_to_series<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    target_meeting_id: String,
) -> Result<(), String> {
    let pool = state.db_manager.pool();

    let series_key =
        MeetingsRepository::link_meeting_to_series(pool, &meeting_id, &target_meeting_id)
            .await
            .map_err(|e| format!("Could not link the meeting: {e}"))?;

    // Invalidate every cached brief in the series (incl. other scheduled occurrences the
    // background pass already briefed) so the next open regenerates with the new prior.
    let dropped = MeetingBriefsRepository::delete_for_series(pool, &series_key)
        .await
        .map_err(|e| format!("Linked, but could not refresh cached briefs: {e}"))?;
    info!(
        "prep: linked {meeting_id} into series {series_key}; invalidated {dropped} cached brief(s)"
    );

    // Warm the target's brief right away so the open Prep tab fills in.
    cancel_run(&target_meeting_id);
    spawn_generation(&app, target_meeting_id, false);
    // Nudge any open Prep views to re-read (the linked-meetings list changed even where no
    // brief regeneration happens).
    let _ = app.emit("prep-briefs-updated", ());
    Ok(())
}

/// Remove `meeting_id`'s manual series link (the Prep tab's unlink affordance). Mirrors the
/// link command's invalidation: the series' cached briefs are deleted so the shrunken prior
/// set takes effect on the next open / background pass.
#[tauri::command]
pub async fn api_unlink_meeting_from_series<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    let pool = state.db_manager.pool();

    // Read the key BEFORE deleting the row — it scopes the brief invalidation.
    let series_key = MeetingsRepository::get_series_link(pool, &meeting_id)
        .await
        .map_err(|e| format!("{e}"))?;
    let removed = MeetingsRepository::unlink_meeting_from_series(pool, &meeting_id)
        .await
        .map_err(|e| format!("Could not unlink the meeting: {e}"))?;

    if removed {
        if let Some(key) = series_key {
            let dropped = MeetingBriefsRepository::delete_for_series(pool, &key)
                .await
                .map_err(|e| format!("Unlinked, but could not refresh cached briefs: {e}"))?;
            info!(
                "prep: unlinked {meeting_id} from series {key}; invalidated {dropped} cached brief(s)"
            );
        }
        let _ = app.emit("prep-briefs-updated", ());
    }
    Ok(())
}

/// Mint (or return) the `scheduled` prep meeting for one upcoming calendar occurrence, and
/// warm its brief. Called when the user opens an upcoming event's prep from the Today view.
#[tauri::command]
pub async fn api_ensure_scheduled_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    calendar_event_id: String,
    series_key: Option<String>,
    title: String,
    occurrence_start: String,
) -> Result<String, String> {
    let pool = state.db_manager.pool();
    let occurrence = chrono::DateTime::parse_from_rfc3339(&occurrence_start)
        .map_err(|e| format!("Invalid occurrence start: {e}"))?
        .with_timezone(&chrono::Utc);

    let meeting_id = MeetingsRepository::upsert_scheduled_meeting(
        pool,
        &calendar_event_id,
        series_key.as_deref(),
        &title,
        occurrence,
    )
    .await
    .map_err(|e| format!("{e}"))?;

    // Warm the brief in the background (no-op if one is already cached/generating).
    if MeetingBriefsRepository::get(pool, &meeting_id)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        spawn_generation(&app, meeting_id.clone(), false);
    }

    Ok(meeting_id)
}

/// Force a fresh brief regeneration (manual refresh). Cancels any in-flight run first.
#[tauri::command]
pub async fn api_regenerate_prep_brief<R: Runtime>(
    app: AppHandle<R>,
    _state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    cancel_run(&meeting_id);
    spawn_generation(&app, meeting_id, true);
    Ok(())
}

/// Autosave the meeting's prep notes (the agenda). Distinct from `api_save_meeting_notes`
/// (live notes) so prep never lands in `notes_markdown`.
#[tauri::command]
pub async fn api_save_prep_notes(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    prep_markdown: Option<String>,
    prep_json: Option<String>,
) -> Result<bool, String> {
    MeetingNotesRepository::upsert_prep_notes(
        state.db_manager.pool(),
        &meeting_id,
        prep_markdown.as_deref(),
        prep_json.as_deref(),
    )
    .await
    .map_err(|e| format!("{e}"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepNotes {
    pub prep_markdown: Option<String>,
    pub prep_json: Option<String>,
}

/// Load just the meeting's prep notes (for the Prep editor's initial content).
#[tauri::command]
pub async fn api_get_prep_notes(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<PrepNotes, String> {
    match MeetingNotesRepository::get_notes(state.db_manager.pool(), &meeting_id).await {
        Ok(Some(n)) => Ok(PrepNotes {
            prep_markdown: n.prep_markdown,
            prep_json: n.prep_json,
        }),
        Ok(None) => Ok(PrepNotes {
            prep_markdown: None,
            prep_json: None,
        }),
        Err(e) => Err(format!("{e}")),
    }
}
