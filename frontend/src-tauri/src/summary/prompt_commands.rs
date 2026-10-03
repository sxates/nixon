//! Tauri commands for saved summary prompts (specs/0079). Logic lives in `*_impl` fns that
//! take a pool so tests need no Tauri runtime; the commands are thin wrappers that surface
//! the user-readable `Display` text of any error.

use crate::database::repositories::summary_prompt::{
    series_key_for, SummaryPrompt, SummaryPromptRepository,
};
use crate::state::AppState;
use crate::summary::prompt_sanitize::{sanitize_prompt_body, sanitize_prompt_name};
use crate::summary::prompts_resolve::{resolve_summary_prompt, PromptSource};
use anyhow::{anyhow, Result};
use sqlx::SqlitePool;

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct MeetingPromptState {
    pub source: PromptSource,
    pub prompt_id: Option<String>,
    pub prompt_name: Option<String>,
    pub extract_action_items: bool,
    /// The meeting's one-off text as stored (cleaned at save time, not escaped).
    pub custom_body: Option<String>,
    pub custom_extract_action_items: Option<bool>,
    pub has_series: bool,
}

fn gone() -> anyhow::Error {
    anyhow!("That prompt no longer exists.")
}

fn no_meeting() -> anyhow::Error {
    anyhow!("That meeting no longer exists.")
}

pub(crate) async fn list_summary_prompts_impl(pool: &SqlitePool) -> Result<Vec<SummaryPrompt>> {
    SummaryPromptRepository::list(pool).await
}

pub(crate) async fn save_summary_prompt_impl(
    pool: &SqlitePool,
    id: Option<String>,
    name: &str,
    body: &str,
    extract_action_items: bool,
) -> Result<SummaryPrompt> {
    let name = sanitize_prompt_name(name)?;
    let body = sanitize_prompt_body(body)?;
    match id {
        None => {
            SummaryPromptRepository::create(pool, &name, &body, extract_action_items, true).await
        }
        Some(id) => {
            // Hidden (series-only) prompts are not editable through the library.
            match SummaryPromptRepository::get(pool, &id).await? {
                Some(p) if p.in_library => {}
                _ => return Err(gone()),
            }
            SummaryPromptRepository::update(pool, &id, &name, &body, extract_action_items)
                .await?
                .ok_or_else(gone)
        }
    }
}

pub(crate) async fn delete_summary_prompt_impl(pool: &SqlitePool, id: &str) -> Result<()> {
    SummaryPromptRepository::delete(pool, id).await
}

pub(crate) async fn set_default_summary_prompt_impl(pool: &SqlitePool, id: &str) -> Result<()> {
    if SummaryPromptRepository::set_default(pool, id).await? {
        Ok(())
    } else {
        Err(gone())
    }
}

pub(crate) async fn get_meeting_prompt_state_impl(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<MeetingPromptState> {
    let fields = SummaryPromptRepository::meeting_fields(pool, meeting_id).await?;
    let resolved = resolve_summary_prompt(pool, meeting_id).await;
    let has_series = fields
        .as_ref()
        .is_some_and(|f| series_key_for(f.calendar_series_key.as_deref(), &f.title).is_some());
    Ok(MeetingPromptState {
        source: resolved.source,
        prompt_id: resolved.prompt_id,
        prompt_name: resolved.prompt_name,
        extract_action_items: resolved.extract_action_items,
        custom_body: fields
            .as_ref()
            .and_then(|f| f.custom_summary_prompt.clone()),
        custom_extract_action_items: fields.and_then(|f| f.custom_extract_action_items),
        has_series,
    })
}

pub(crate) async fn set_meeting_summary_prompt_impl(
    pool: &SqlitePool,
    meeting_id: &str,
    prompt_id: Option<String>,
    apply_to_series: bool,
) -> Result<()> {
    let fields = SummaryPromptRepository::meeting_fields(pool, meeting_id)
        .await?
        .ok_or_else(no_meeting)?;
    let Some(prompt_id) = prompt_id else {
        SummaryPromptRepository::set_meeting_prompt_id(pool, meeting_id, None).await?;
        return Ok(());
    };
    match SummaryPromptRepository::get(pool, &prompt_id).await? {
        Some(p) if p.in_library => {}
        _ => return Err(gone()),
    }
    SummaryPromptRepository::set_meeting_prompt_id(pool, meeting_id, Some(&prompt_id)).await?;
    SummaryPromptRepository::set_meeting_custom_prompt(pool, meeting_id, None, true).await?;
    if apply_to_series {
        // Best-effort stickiness: without a usable series key the pick still applies here.
        if let Some(key) = series_key_for(fields.calendar_series_key.as_deref(), &fields.title) {
            SummaryPromptRepository::set_series_prompt(pool, &key, &prompt_id).await?;
        }
    }
    Ok(())
}

pub(crate) async fn set_meeting_custom_prompt_impl(
    pool: &SqlitePool,
    meeting_id: &str,
    body: Option<String>,
    extract_action_items: bool,
) -> Result<()> {
    let cleaned = match body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        Some(raw) => Some(sanitize_prompt_body(raw)?),
        None => None,
    };
    if SummaryPromptRepository::set_meeting_custom_prompt(
        pool,
        meeting_id,
        cleaned.as_deref(),
        extract_action_items,
    )
    .await?
    {
        Ok(())
    } else {
        Err(no_meeting())
    }
}

pub(crate) async fn save_custom_prompt_followup_impl(
    pool: &SqlitePool,
    meeting_id: &str,
    name: Option<String>,
    to_library: bool,
    to_series: bool,
) -> Result<SummaryPrompt> {
    if !to_library && !to_series {
        return Err(anyhow!("Choose where to save the prompt."));
    }
    let f = SummaryPromptRepository::meeting_fields(pool, meeting_id)
        .await?
        .ok_or_else(no_meeting)?;
    let body = match f.custom_summary_prompt.as_deref() {
        Some(raw) => {
            sanitize_prompt_body(raw).map_err(|_| anyhow!("Write a custom prompt first."))?
        }
        None => return Err(anyhow!("Write a custom prompt first.")),
    };
    let extract = f.custom_extract_action_items.unwrap_or(true);
    // Validate the series key before any write so a failure leaves nothing behind.
    let series_key = if to_series {
        Some(
            series_key_for(f.calendar_series_key.as_deref(), &f.title)
                .ok_or_else(|| anyhow!("This meeting isn't part of a recurring series."))?,
        )
    } else {
        None
    };
    let title = f.title.trim();
    let prompt_name = if to_library {
        match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => sanitize_prompt_name(n)?,
            None => {
                let base = if title.is_empty() {
                    "Custom prompt"
                } else {
                    title
                };
                SummaryPromptRepository::unique_name(pool, base).await?
            }
        }
    } else {
        // Never shown; the unique name index only covers library rows.
        format!("{title} (series)").chars().take(60).collect()
    };
    // Sequential writes: a failure after `create` leaves at worst an orphan hidden prompt.
    let prompt =
        SummaryPromptRepository::create(pool, &prompt_name, &body, extract, to_library).await?;
    if let Some(key) = series_key {
        SummaryPromptRepository::set_series_prompt(pool, &key, &prompt.id).await?;
    }
    SummaryPromptRepository::set_meeting_prompt_id(pool, meeting_id, Some(&prompt.id)).await?;
    SummaryPromptRepository::set_meeting_custom_prompt(pool, meeting_id, None, true).await?;
    Ok(prompt)
}

#[tauri::command]
pub async fn api_list_summary_prompts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SummaryPrompt>, String> {
    list_summary_prompts_impl(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_save_summary_prompt(
    state: tauri::State<'_, AppState>,
    id: Option<String>,
    name: String,
    body: String,
    extract_action_items: bool,
) -> Result<SummaryPrompt, String> {
    save_summary_prompt_impl(
        state.db_manager.pool(),
        id,
        &name,
        &body,
        extract_action_items,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_delete_summary_prompt(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    delete_summary_prompt_impl(state.db_manager.pool(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_set_default_summary_prompt(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    set_default_summary_prompt_impl(state.db_manager.pool(), &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_get_meeting_prompt_state(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<MeetingPromptState, String> {
    get_meeting_prompt_state_impl(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_set_meeting_summary_prompt(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    prompt_id: Option<String>,
    apply_to_series: bool,
) -> Result<(), String> {
    set_meeting_summary_prompt_impl(
        state.db_manager.pool(),
        &meeting_id,
        prompt_id,
        apply_to_series,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_set_meeting_custom_prompt(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    body: Option<String>,
    extract_action_items: bool,
) -> Result<(), String> {
    set_meeting_custom_prompt_impl(
        state.db_manager.pool(),
        &meeting_id,
        body,
        extract_action_items,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn api_save_custom_prompt_followup(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    name: Option<String>,
    to_library: bool,
    to_series: bool,
) -> Result<SummaryPrompt, String> {
    save_custom_prompt_followup_impl(
        state.db_manager.pool(),
        &meeting_id,
        name,
        to_library,
        to_series,
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
