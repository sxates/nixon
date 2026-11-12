//! To-do list / inbox commands (specs/0088). Every mutation emits `action-items-updated`.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::ACTION_ITEMS_UPDATED_EVENT;
use crate::database::repositories::action_item::{ActionItem, ActionItemsRepository};
use crate::database::repositories::action_item_todo::{
    ActionItemDetail, ActionItemTodoRepository as Todo, TodoRow, TodoView,
};
use crate::state::AppState;

const GONE: &str = "That action item no longer exists";

fn emit_updated<R: Runtime>(app: &AppHandle<R>, meeting_id: Option<&str>) {
    let _ = app.emit(
        ACTION_ITEMS_UPDATED_EVENT,
        serde_json::json!({ "meeting_id": meeting_id.unwrap_or_default() }),
    );
}

#[tauri::command]
pub async fn api_list_todo_view<R: Runtime>(
    app: AppHandle<R>,
    view: String,
) -> Result<Vec<TodoRow>, String> {
    let view = TodoView::parse(view.trim())
        .ok_or_else(|| format!("Unknown view '{view}' — use list, inbox, done, or dismissed"))?;
    let state = app.state::<AppState>();
    Todo::list_view(state.db_manager.pool(), view)
        .await
        .map_err(|e| format!("Failed to load your to-dos: {e}"))
}

#[tauri::command]
pub async fn api_inbox_count<R: Runtime>(app: AppHandle<R>) -> Result<i64, String> {
    let state = app.state::<AppState>();
    Todo::inbox_count(state.db_manager.pool())
        .await
        .map_err(|e| format!("Failed to count inbox suggestions: {e}"))
}

#[tauri::command]
pub async fn api_triage_action_item<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    action: String,
) -> Result<ActionItem, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let res = match action.trim() {
        "add_to_list" | "adopt" => Todo::add_to_list(pool, &id).await,
        "restore" => Todo::restore(pool, &id).await,
        "dismiss" => ActionItemsRepository::set_status(pool, &id, "dismissed").await,
        other => {
            return Err(format!(
                "Unknown action '{other}' — use add_to_list, adopt, dismiss, or restore"
            ))
        }
    };
    let item = res
        .map_err(|e| format!("Failed to update action item: {e}"))?
        .ok_or_else(|| GONE.to_string())?;
    emit_updated(&app, item.meeting_id.as_deref());
    Ok(item)
}

#[tauri::command]
pub async fn api_get_action_item_detail<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<ActionItemDetail, String> {
    let state = app.state::<AppState>();
    Todo::get_detail(state.db_manager.pool(), &id)
        .await
        .map_err(|e| format!("Failed to load action item: {e}"))?
        .ok_or_else(|| GONE.to_string())
}

#[tauri::command]
pub async fn api_set_action_item_notes<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    markdown: Option<String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    if !Todo::set_notes(pool, &id, markdown.as_deref())
        .await
        .map_err(|e| format!("Failed to save notes: {e}"))?
    {
        return Err(GONE.to_string());
    }
    let meeting_id = ActionItemsRepository::get(pool, &id)
        .await
        .ok()
        .flatten()
        .and_then(|i| i.meeting_id);
    emit_updated(&app, meeting_id.as_deref());
    Ok(())
}

#[tauri::command]
pub async fn api_dismiss_suggestions_older_than<R: Runtime>(
    app: AppHandle<R>,
    days: i64,
) -> Result<Vec<String>, String> {
    if ![14, 30, 60].contains(&days) {
        return Err("Choose 14, 30, or 60 days".to_string());
    }
    let state = app.state::<AppState>();
    let ids = Todo::dismiss_suggestions_older_than(state.db_manager.pool(), days)
        .await
        .map_err(|e| format!("Failed to dismiss old suggestions: {e}"))?;
    if !ids.is_empty() {
        emit_updated(&app, None);
    }
    Ok(ids)
}
