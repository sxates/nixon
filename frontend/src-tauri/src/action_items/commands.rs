//! Tauri commands for action items (specs/0034).
//!
//! Frontend → Rust surface for the per-meeting section and the task hub:
//! - [`api_get_action_items`]       — per-meeting section (all statuses/sources).
//! - [`api_list_action_items`]      — hub query, joined to meeting title/date; default
//!   filter `status = 'open'` (pass `status: "all"` for everything).
//! - [`api_create_action_item`]     — manual add; `meeting_id` nullable → standalone to-do.
//! - [`api_update_action_item`]     — content edit; sets `user_edited = 1` (→ protected)
//!   and recomputes `content_key`.
//! - [`api_set_action_item_status`] — open/completed/dismissed; manages `completed_at`;
//!   any user status change (even back to open) sets `user_edited = 1` → the row is
//!   permanently protected from re-extraction.
//! - [`api_delete_action_item`]     — hard delete (UI offers it for MANUAL items only —
//!   deleting an extracted item would just get re-proposed; dismiss is the persistent no).
//! - [`api_extract_action_items`]   — manual "Scan again" / retroactive extraction over
//!   the stored summary; returns the number of rows the run inserted or updated (0 on
//!   any no-op — unchanged summary, run already in flight, nothing new to write).
//! - [`api_reorder_action_items`]   — persist a manual drag order (specs/0038 WS1.b):
//!   dense `sort_order` 0,1,2,… for the given ids, one transaction.
//! - [`api_bulk_set_action_item_status`] — bulk dismiss/status over a person / not-me /
//!   explicit-id set (specs/0038 WS1.e); returns the affected count for an Undo toast.
//!
//! All return user-actionable error strings; DB I/O lives in `ActionItemsRepository`;
//! extraction orchestration in [`super::run_extraction`].

use serde::Deserialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::action_items::diff::{self, ResolvedCandidate};
use crate::database::repositories::action_item::{
    ActionItem, ActionItemFilters, ActionItemWithMeeting, ActionItemsRepository, BulkStatusFilter,
};
use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::database::repositories::setting::SettingsRepository;
use crate::database::repositories::summary::SummaryProcessesRepository;
use crate::state::AppState;
use crate::summary::llm_gate::{with_priority, Priority};
use crate::summary::provider_config::{resolve_provider_config, ProviderConfig};

const VALID_STATUSES: [&str; 3] = ["open", "completed", "dismissed"];

/// The inputs `run_extraction` needs that a task id alone cannot carry (specs/0063 W3):
/// the stored summary markdown, the meeting's user notes, and the resolved provider
/// config. Shared by the manual "Scan again" command and the queue's per-task Retry
/// (`llm_activity::retry`) so the stored-summary JSON shape (`result` → `markdown`) has
/// exactly one reader — duplicating this reach-into-JSON would drift silently the first
/// time that shape changes.
pub struct ExtractionInputs {
    pub summary_markdown: String,
    pub user_notes: Option<String>,
    pub provider_config: ProviderConfig,
}

/// Reload everything `run_extraction` needs for `meeting_id` from the DB. Errors are
/// user-actionable strings — no summary yet, no model configured, etc. — since both call
/// sites surface them straight to the user.
pub async fn load_extraction_inputs(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<ExtractionInputs, String> {
    // The stored summary markdown is the extraction source (specs/0034: extraction is
    // event-driven off the artifact it extracts from).
    let process = SummaryProcessesRepository::get_summary_data(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load the meeting's summary: {e}"))?;
    let summary_markdown = process
        .and_then(|p| p.result)
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| {
            v.get("markdown")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| {
            "Generate a summary first — action items are extracted from the summary".to_string()
        })?;

    let user_notes = MeetingNotesRepository::get_notes(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load the meeting's notes: {e}"))?
        .and_then(|note| note.notes_markdown)
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());

    // Same provider posture as the summary runs: the configured summary model, resolved
    // through the SAME credential resolver the summary path uses (never drifts).
    let config = SettingsRepository::get_model_config(pool)
        .await
        .map_err(|e| format!("Failed to load model configuration: {e}"))?
        .ok_or_else(|| {
            "No summary model is configured. Choose a provider and model in Settings first."
                .to_string()
        })?;
    let provider_config = resolve_provider_config(pool, &config.provider, &config.model)
        .await
        .map_err(|e| format!("{e:#}"))?;

    Ok(ExtractionInputs {
        summary_markdown,
        user_notes,
        provider_config,
    })
}

/// All action items for a meeting (any status/source), oldest first.
#[tauri::command]
pub async fn api_get_action_items<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Vec<ActionItem>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    ActionItemsRepository::get_for_meeting(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load action items: {e}"))
}

/// Cross-meeting task-hub query. `status` defaults to `'open'`; pass `"all"` to disable
/// the status filter. `person_id` filters by resolved assignee; `mine_only` filters to
/// items assigned to the app owner (`assignee_is_self`).
#[tauri::command]
pub async fn api_list_action_items<R: Runtime>(
    app: AppHandle<R>,
    status: Option<String>,
    person_id: Option<String>,
    mine_only: Option<bool>,
) -> Result<Vec<ActionItemWithMeeting>, String> {
    let status = match status.as_deref().map(str::trim) {
        None | Some("") => Some("open".to_string()),
        Some("all") => None,
        Some(s) if VALID_STATUSES.contains(&s) => Some(s.to_string()),
        Some(other) => {
            return Err(format!(
                "Unknown status filter '{other}' — use open, completed, dismissed, or all"
            ))
        }
    };

    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    ActionItemsRepository::list(
        pool,
        ActionItemFilters {
            status: status.as_deref(),
            person_id: person_id
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty()),
            mine_only: mine_only.unwrap_or(false),
        },
    )
    .await
    .map_err(|e| format!("Failed to load action items: {e}"))
}

/// Create a manual action item (`source = 'manual'`, protected from re-extraction by
/// construction). `meeting_id` is optional — `None` creates a standalone to-do from the
/// hub. At most one of `assignee_person_id` / `assignee_is_self` should be set.
#[tauri::command]
pub async fn api_create_action_item<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: Option<String>,
    description: String,
    assignee_person_id: Option<String>,
    assignee_is_self: Option<bool>,
    due_hint: Option<String>,
    due_date: Option<String>,
) -> Result<ActionItem, String> {
    let description = description.trim().to_string();
    if description.is_empty() {
        return Err("A description is required to create an action item".to_string());
    }

    let assignee_is_self = assignee_is_self.unwrap_or(false);
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let key = diff::content_key(&description);
    ActionItemsRepository::create(
        pool,
        meeting_id
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty()),
        &ResolvedCandidate {
            description,
            assignee_person_id: assignee_person_id.filter(|p| !p.trim().is_empty()),
            assignee_is_self,
            assignee_raw: None,
            due_hint: due_hint
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
            due_date: due_date
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
        },
        "manual",
        &key,
    )
    .await
    .map_err(|e| format!("Failed to create action item: {e}"))
}

/// Edit an item's content. Any content change sets `user_edited = 1` (the row becomes
/// protected — re-extraction will never modify, delete, or duplicate it) and recomputes
/// `content_key` when the description changes.
///
/// Assignee semantics (first match wins): `assignee_clear` → unassigned;
/// `assignee_person_id` → that person; `assignee_raw` → free-text display name (no
/// person link — the roster-gap fix-up path); `assignee_is_self: true` → the app owner.
/// The self flag is SET-ONLY (`false` is ignored — un-assigning goes through
/// `assignee_clear` or one of the other forms). Omitted fields keep their current
/// values; `due_hint: ""` clears the hint and `due_date: ""` clears the structured date
/// (the hint is verbatim provenance, `due_date` is the sortable ISO value — WS1.a).
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri IPC arg shape — a struct would change the frontend contract
pub async fn api_update_action_item<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    description: Option<String>,
    assignee_person_id: Option<String>,
    assignee_is_self: Option<bool>,
    assignee_raw: Option<String>,
    assignee_clear: Option<bool>,
    due_hint: Option<String>,
    due_date: Option<String>,
) -> Result<ActionItem, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let existing = ActionItemsRepository::get(pool, &id)
        .await
        .map_err(|e| format!("Failed to load action item: {e}"))?
        .ok_or_else(|| "That action item no longer exists".to_string())?;

    let description = match description.map(|d| d.trim().to_string()) {
        Some(d) if d.is_empty() => {
            return Err("An action item's description can't be empty".to_string())
        }
        Some(d) => d,
        None => existing.description.clone(),
    };
    let content_key = diff::content_key(&description);

    // Assignee merge: explicit clear > person > free-text raw > set-self > keep current.
    let (person_id, is_self, raw) = if assignee_clear.unwrap_or(false) {
        (None, false, None)
    } else if let Some(pid) = assignee_person_id.filter(|p| !p.trim().is_empty()) {
        (Some(pid), false, None)
    } else if let Some(raw_name) = assignee_raw
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
    {
        // Free-text assignee (roster gap): display name only, no person link, not self.
        (None, false, Some(raw_name))
    } else if assignee_is_self == Some(true) {
        // Set-self-only: `Some(false)` is treated like an omitted flag (the frontend
        // un-assigns via `assignee_clear` or by picking another form).
        (None, true, None)
    } else {
        (
            existing.assignee_person_id.clone(),
            existing.assignee_is_self,
            existing.assignee_raw.clone(),
        )
    };

    // Due hint / structured date: omitted keeps, "" clears, anything else sets. The
    // hint stays as provenance; `due_date` is the sortable structured value (WS1.a).
    let due_hint = match due_hint.map(|d| d.trim().to_string()) {
        None => existing.due_hint.clone(),
        Some(d) if d.is_empty() => None,
        Some(d) => Some(d),
    };
    let due_date = match due_date.map(|d| d.trim().to_string()) {
        None => existing.due_date.clone(),
        Some(d) if d.is_empty() => None,
        Some(d) => Some(d),
    };

    ActionItemsRepository::update_content(
        pool,
        &id,
        &description,
        person_id.as_deref(),
        is_self,
        raw.as_deref(),
        due_hint.as_deref(),
        due_date.as_deref(),
        &content_key,
    )
    .await
    .map_err(|e| format!("Failed to update action item: {e}"))?
    .ok_or_else(|| "That action item no longer exists".to_string())
}

/// Flip an item's status: `'open' | 'completed' | 'dismissed'`. Completing stamps
/// `completed_at`. Only users reach this command (the machine never flips status), so
/// ANY transition — including un-checking back to 'open' — also sets `user_edited = 1`:
/// once the user has touched a row it is permanently protected from re-extraction
/// (dismissing an extracted item is the persistent "don't resurrect this"; the 2026-07
/// incident showed un-checking must not be the one transition that forfeits protection).
#[tauri::command]
pub async fn api_set_action_item_status<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    status: String,
) -> Result<ActionItem, String> {
    let status = status.trim().to_lowercase();
    if !VALID_STATUSES.contains(&status.as_str()) {
        return Err(format!(
            "Unknown status '{status}' — use open, completed, or dismissed"
        ));
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    ActionItemsRepository::set_status(pool, &id, &status)
        .await
        .map_err(|e| format!("Failed to update action item status: {e}"))?
        .ok_or_else(|| "That action item no longer exists".to_string())
}

/// Hard-delete an action item. The UI offers this for MANUAL items only — deleting an
/// extracted item would just get re-proposed on the next extraction (that's what dismiss
/// is for) — but the command doesn't enforce it (power-user escape hatch).
#[tauri::command]
pub async fn api_delete_action_item<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    ActionItemsRepository::delete(pool, &id)
        .await
        .map_err(|e| format!("Failed to delete action item: {e}"))
}

/// Manual extraction trigger ("Scan again" / retroactive extraction for pre-0034
/// meetings): runs the same extract+diff pass as the background trigger against the
/// STORED summary + notes, using the currently configured summary provider/model.
///
/// Returns the number of action-item rows this run inserted or updated. `0` = nothing
/// changed: the summary+notes are unchanged since the last successful extraction (ledger
/// fingerprint match), another extraction for this meeting is already in flight, or
/// every candidate was absorbed by existing user-owned rows.
#[tauri::command]
pub async fn api_extract_action_items<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<u32, String> {
    let meeting_id = meeting_id.trim().to_string();
    if meeting_id.is_empty() {
        return Err("meeting_id cannot be empty".to_string());
    }

    let state = app.state::<AppState>();
    let pool = state.db_manager.pool().clone();

    let ExtractionInputs {
        summary_markdown,
        user_notes,
        provider_config,
    } = load_extraction_inputs(&pool, &meeting_id).await?;

    // specs/0056 W2: "Scan again" is a click with a spinner — interactive priority, unlike the
    // post-summary background extraction that shares this code path.
    with_priority(
        Priority::Interactive,
        super::run_extraction(
            &app,
            &pool,
            &meeting_id,
            &summary_markdown,
            user_notes.as_deref(),
            &provider_config,
        ),
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

/// Persist a manual drag order for the task hub (specs/0038 WS1.b). `ordered_ids` is the
/// full desired order of the affected rows; the repo writes dense `sort_order` values
/// (0, 1, 2, …) in one transaction. Ids that no longer exist are silently skipped.
/// Returns the number of rows written. Does NOT protect a row from re-extraction — order
/// is a view-only concern, orthogonal to content.
#[tauri::command]
pub async fn api_reorder_action_items<R: Runtime>(
    app: AppHandle<R>,
    ordered_ids: Vec<String>,
) -> Result<u64, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    ActionItemsRepository::reorder(pool, &ordered_ids)
        .await
        .map_err(|e| format!("Failed to reorder action items: {e}"))
}

/// How `api_bulk_set_action_item_status` selects its target rows (specs/0038 WS1.e). A
/// single tagged object over IPC:
/// - `{ "mode": "person", "personId": "person-…" }` — every OPEN item assigned to that
///   `people` row;
/// - `{ "mode": "notSelf" }` — every OPEN item NOT assigned to me (`assignee_is_self = 0`;
///   covers other people AND unassigned/raw-name rows). "Me" is the self flag, never a
///   `people` FK, so my own items are never touched;
/// - `{ "mode": "ids", "ids": ["ai-…", …] }` — exactly these rows, any status (the general
///   primitive; also how the frontend Undoes a bulk dismiss by re-`open`-ing the same ids).
#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum BulkFilterArg {
    Person {
        #[serde(rename = "personId")]
        person_id: String,
    },
    NotSelf,
    Ids {
        ids: Vec<String>,
    },
}

/// Bulk status change over a selected set in ONE transaction (specs/0038 WS1.e). Primary
/// use: dismiss items owned by other people ("Dismiss all not assigned to me" / "Dismiss
/// all from {person}"). Every touched row is stamped `user_edited = 1` — so a dismissed
/// extracted item stays dismissed across re-extraction (0034's protected/pristine rule) —
/// and `completed_at` is managed like the per-item path. Returns the affected row count
/// for the Undo toast; the frontend Undoes by re-`open`-ing those ids (the `ids` mode).
#[tauri::command]
pub async fn api_bulk_set_action_item_status<R: Runtime>(
    app: AppHandle<R>,
    filter: BulkFilterArg,
    status: String,
) -> Result<u64, String> {
    let status = status.trim().to_lowercase();
    if !VALID_STATUSES.contains(&status.as_str()) {
        return Err(format!(
            "Unknown status '{status}' — use open, completed, or dismissed"
        ));
    }

    let repo_filter = match &filter {
        BulkFilterArg::Person { person_id } => {
            let pid = person_id.trim();
            if pid.is_empty() {
                return Err("A person is required to dismiss their action items".to_string());
            }
            BulkStatusFilter::Person(pid)
        }
        BulkFilterArg::NotSelf => BulkStatusFilter::NotSelf,
        BulkFilterArg::Ids { ids } => BulkStatusFilter::Ids(ids),
    };

    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    ActionItemsRepository::bulk_set_status(pool, &repo_filter, &status)
        .await
        .map_err(|e| format!("Failed to update action items: {e}"))
}
