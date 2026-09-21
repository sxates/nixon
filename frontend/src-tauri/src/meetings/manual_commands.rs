//! specs/0069 W3 — the IPC surface for manually added meetings. Separate from
//! `meetings/commands.rs` (673 lines) to stay under the 800-line cap.

use tauri::State;

use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

/// Parse an RFC-3339 instant from the frontend, naming the field in the error so a bad
/// value is debuggable from the toast alone.
fn parse_instant(field: &str, value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| format!("Invalid {field}: {e}"))
}

#[tauri::command]
pub async fn api_create_manual_meeting(
    state: State<'_, AppState>,
    title: String,
    starts_at: String,
    ends_at: Option<String>,
    join_url: Option<String>,
) -> Result<String, String> {
    let starts = parse_instant("start time", &starts_at)?;
    let ends = match ends_at.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(parse_instant("end time", s)?),
        None => None,
    };
    let id = MeetingsRepository::create_manual_scheduled(
        state.db_manager.pool(),
        &title,
        starts,
        ends,
        join_url.as_deref(),
    )
    .await
    .map_err(|e| format!("Could not add the meeting: {e}"))?;
    log::info!("added a manual meeting {id} at {starts} (specs/0069 W3)");
    Ok(id)
}

#[tauri::command]
pub async fn api_update_manual_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
    title: String,
    starts_at: String,
    ends_at: Option<String>,
    join_url: Option<String>,
) -> Result<(), String> {
    let starts = parse_instant("start time", &starts_at)?;
    let ends = match ends_at.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(parse_instant("end time", s)?),
        None => None,
    };
    let updated = MeetingsRepository::update_manual_scheduled(
        state.db_manager.pool(),
        &meeting_id,
        &title,
        starts,
        ends,
        join_url.as_deref(),
    )
    .await
    .map_err(|e| format!("Could not update the meeting: {e}"))?;
    if !updated {
        return Err("This meeting has been recorded — edit it from the meeting page".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn api_delete_manual_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    let deleted = MeetingsRepository::delete_manual_scheduled(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Could not remove the meeting: {e}"))?;
    if !deleted {
        return Err("This meeting has been recorded — delete it from the meeting page".into());
    }
    Ok(())
}
