//! Tauri commands for Topics (specs/0089). Every write emits `topics-updated`; errors are
//! user-readable strings.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::evidence::{blocks, for_entry, load_segments, topic_terms};
use super::limits::{DESCRIBE_MEETINGS_CAP, OPEN_DECISIONS_CAP};
use super::links::{self, LinkKind, TopicLink};
use super::model::{EntryBody, Topic, TopicEntry, TopicSuggestion, TopicSummary};
use super::repo::{emit_updated, TopicsRepository as Topics};
use super::{describe, jobs, ledger, writer};
use crate::action_items::commands::load_extraction_inputs;
use crate::action_items::ACTION_ITEMS_UPDATED_EVENT;
use crate::database::repositories::action_item_todo::{ActionItemTodoRepository as Todo, TodoRow};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;
use crate::summary::llm_gate::{with_priority, Priority};

const TOPIC_GONE: &str = "Topic not found.";
const ENTRY_GONE: &str = "That entry no longer exists.";
const EDITED: &str = "Edited entries can't be rewritten.";
pub const SERIES_ONLY: &str = "Only series entries can be rewritten.";

fn pool_of<R: Runtime>(app: &AppHandle<R>) -> SqlitePool {
    app.state::<AppState>().db_manager.pool().clone()
}

fn db(what: &'static str) -> impl Fn(anyhow::Error) -> String {
    move |e| format!("{what}: {e:#}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicDetail {
    pub topic: Topic,
    pub links: Vec<TopicLink>,
    pub entries: Vec<TopicEntry>,
    pub suggestions: Vec<TopicSuggestion>,
    pub todos: Vec<TodoRow>,
}

async fn require_topic(pool: &SqlitePool, id: &str) -> Result<Topic, String> {
    Topics::get(pool, id)
        .await
        .map_err(db("Couldn't load the topic"))?
        .ok_or_else(|| TOPIC_GONE.to_string())
}

/// The topic an entry belongs to, for the event payload.
async fn entry_topic(pool: &SqlitePool, entry_id: &str) -> Result<String, String> {
    sqlx::query_scalar::<_, String>("SELECT topic_id FROM topic_entries WHERE id = ?")
        .bind(entry_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Couldn't load the entry: {e}"))?
        .ok_or_else(|| ENTRY_GONE.to_string())
}

#[tauri::command]
pub async fn api_list_topics<R: Runtime>(
    app: AppHandle<R>,
    status: Option<String>,
) -> Result<Vec<TopicSummary>, String> {
    Topics::list(&pool_of(&app), status.as_deref())
        .await
        .map_err(db("Couldn't load topics"))
}

#[tauri::command]
pub async fn api_get_topic<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<TopicDetail, String> {
    let pool = pool_of(&app);
    let topic = require_topic(&pool, &id).await?;
    let load = db("Couldn't load the topic");
    Ok(TopicDetail {
        links: links::links_for_topic(&pool, &id).await.map_err(&load)?,
        entries: ledger::entries_for_topic(&pool, &id).await.map_err(&load)?,
        suggestions: ledger::pending_suggestions(&pool, &id)
            .await
            .map_err(&load)?,
        todos: Todo::list_for_topic(&pool, &id)
            .await
            .map_err(|e| format!("Couldn't load the topic's to-dos: {e}"))?,
        topic,
    })
}

#[tauri::command]
pub async fn api_create_topic<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    description: Option<String>,
    link_meeting_id: Option<String>,
    link_series: bool,
    backfill: bool,
) -> Result<Topic, String> {
    let pool = pool_of(&app);
    let topic = Topics::create(&pool, &name, description.as_deref().unwrap_or(""))
        .await
        .map_err(|e| format!("{e:#}"))?;
    if let Some(meeting_id) = link_meeting_id.as_deref() {
        let scope = if link_series { "series" } else { "meeting" };
        if let Err(e) = link_and_backfill(&app, &pool, &topic.id, meeting_id, scope, backfill).await
        {
            // Don't leave a half-made topic behind when the requested link failed.
            let _ = Topics::delete(&pool, &topic.id).await;
            return Err(e);
        }
    }
    emit_updated(&app, Some(&topic.id));
    Ok(topic)
}

async fn link_and_backfill<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    topic_id: &str,
    meeting_id: &str,
    scope: &str,
    backfill: bool,
) -> Result<TopicLink, String> {
    let link = match scope {
        "meeting" => links::link_meeting(pool, topic_id, meeting_id).await,
        "series" => links::link_series_of(pool, topic_id, meeting_id).await,
        other => {
            return Err(format!(
                "Unknown scope '{other}' — use \"meeting\" or \"series\""
            ))
        }
    }
    .map_err(|e| format!("{e:#}"))?;
    // The seed meeting is always queued when summarised; `backfill` adds the series' earlier
    // meetings.
    let ids = links::entry_targets(pool, &link, meeting_id, backfill, chrono::Utc::now())
        .await
        .map_err(db("Couldn't find the meetings to add"))?;
    if !ids.is_empty() {
        jobs::spawn_backfill(app.clone(), pool.clone(), topic_id.to_string(), ids);
    }
    Ok(link)
}

#[tauri::command]
pub async fn api_update_topic<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    status: Option<String>,
) -> Result<Topic, String> {
    let topic = Topics::update(
        &pool_of(&app),
        &id,
        name.as_deref(),
        description.as_deref(),
        status.as_deref(),
    )
    .await
    .map_err(|e| format!("{e:#}"))?
    .ok_or_else(|| TOPIC_GONE.to_string())?;
    emit_updated(&app, Some(&id));
    Ok(topic)
}

#[tauri::command]
pub async fn api_set_topic_notes<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    notes_markdown: String,
    notes_json: Option<String>,
) -> Result<(), String> {
    let saved = Topics::set_notes(&pool_of(&app), &id, &notes_markdown, notes_json.as_deref())
        .await
        .map_err(db("Couldn't save your notes"))?;
    if !saved {
        return Err(TOPIC_GONE.to_string());
    }
    emit_updated(&app, Some(&id));
    Ok(())
}

#[tauri::command]
pub async fn api_delete_topic<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let gone = Topics::delete(&pool_of(&app), &id)
        .await
        .map_err(db("Couldn't delete the topic"))?;
    if !gone {
        return Err(TOPIC_GONE.to_string());
    }
    emit_updated(&app, Some(&id));
    // Tagged to-dos were untagged by the delete.
    let _ = app.emit(
        ACTION_ITEMS_UPDATED_EVENT,
        serde_json::json!({ "meeting_id": "" }),
    );
    Ok(())
}

#[tauri::command]
pub async fn api_link_topic<R: Runtime>(
    app: AppHandle<R>,
    topic_id: String,
    meeting_id: String,
    scope: String,
    backfill: bool,
) -> Result<TopicLink, String> {
    let pool = pool_of(&app);
    let link = link_and_backfill(&app, &pool, &topic_id, &meeting_id, &scope, backfill).await?;
    emit_updated(&app, Some(&topic_id));
    Ok(link)
}

#[tauri::command]
pub async fn api_unlink_topic<R: Runtime>(
    app: AppHandle<R>,
    topic_id: String,
    kind: LinkKind,
    #[allow(non_snake_case)] r#ref: String,
) -> Result<(), String> {
    links::unlink(&pool_of(&app), &topic_id, kind, &r#ref)
        .await
        .map_err(db("Couldn't remove the link"))?;
    emit_updated(&app, Some(&topic_id));
    Ok(())
}

#[tauri::command]
pub async fn api_topics_for_meeting<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Vec<Topic>, String> {
    links::topics_for_meeting(&pool_of(&app), &meeting_id, false)
        .await
        .map_err(db("Couldn't load this meeting's topics"))
}

/// Interactive; returns the draft and never saves it.
#[tauri::command]
pub async fn api_draft_topic_description<R: Runtime>(
    app: AppHandle<R>,
    topic_id: String,
) -> Result<String, String> {
    let pool = pool_of(&app);
    let topic = require_topic(&pool, &topic_id).await?;
    let ids = links::recent_summarised_meetings(&pool, &topic_id, DESCRIBE_MEETINGS_CAP)
        .await
        .map_err(db("Couldn't find the linked meetings"))?;
    let Some(newest) = ids.first() else {
        return Err(describe::NOTHING_LINKED.to_string());
    };
    let config = load_extraction_inputs(&pool, newest).await?.provider_config;
    let mut summaries = Vec::new();
    for id in &ids {
        let Ok(inputs) = load_extraction_inputs(&pool, id).await else {
            continue;
        };
        let title = match MeetingsRepository::get_meeting_metadata(&pool, id).await {
            Ok(Some(m)) => m.title,
            _ => "Meeting".to_string(),
        };
        summaries.push((title, inputs.summary_markdown));
    }
    let dir = app.path().app_data_dir().ok();
    let client = reqwest::Client::new();
    with_priority(
        Priority::Interactive,
        describe::draft(&client, &config, dir.as_ref(), &topic, &summaries),
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn api_edit_topic_entry<R: Runtime>(
    app: AppHandle<R>,
    entry_id: String,
    body: EntryBody,
) -> Result<(), String> {
    let pool = pool_of(&app);
    let topic_id = entry_topic(&pool, &entry_id).await?;
    if !ledger::edit_entry(&pool, &entry_id, body)
        .await
        .map_err(db("Couldn't save the entry"))?
    {
        return Err(ENTRY_GONE.to_string());
    }
    emit_updated(&app, Some(&topic_id));
    Ok(())
}

#[tauri::command]
pub async fn api_revert_topic_entry<R: Runtime>(
    app: AppHandle<R>,
    entry_id: String,
) -> Result<(), String> {
    let pool = pool_of(&app);
    let topic_id = entry_topic(&pool, &entry_id).await?;
    if !ledger::revert_entry(&pool, &entry_id)
        .await
        .map_err(db("Couldn't revert the entry"))?
    {
        return Err("This entry has no original to revert to.".to_string());
    }
    emit_updated(&app, Some(&topic_id));
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RewriteRow {
    topic_id: String,
    kind: String,
    meeting_id: Option<String>,
    edited_at: Option<String>,
    ai_body_json: Option<String>,
}

/// The (topic, source meeting) of an entry "Rewrite from meeting" may act on, or the
/// user-readable reason it can't: series entries only, never once the owner edited it.
pub async fn rewrite_source(pool: &SqlitePool, entry_id: &str) -> Result<(String, String), String> {
    let row: Option<RewriteRow> = sqlx::query_as(
        "SELECT topic_id, kind, meeting_id, edited_at, ai_body_json FROM topic_entries WHERE id = ?",
    )
    .bind(entry_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Couldn't load the entry: {e}"))?;
    let Some(RewriteRow {
        topic_id,
        kind,
        meeting_id,
        edited_at,
        ai_body_json: ai_body,
    }) = row
    else {
        return Err(ENTRY_GONE.to_string());
    };
    if kind != "series" {
        return Err(SERIES_ONLY.to_string());
    }
    if edited_at.is_some() {
        return Err(EDITED.to_string());
    }
    let (Some(meeting_id), Some(_)) = (meeting_id, ai_body) else {
        return Err("This entry has no source meeting to rewrite from.".to_string());
    };
    Ok((topic_id, meeting_id))
}

/// Interactive. Reloads the source meeting's summary, notes and transcript, drafts the entry
/// again and replaces both bodies; see [`rewrite_source`] for which entries qualify.
#[tauri::command]
pub async fn api_rewrite_topic_entry<R: Runtime>(
    app: AppHandle<R>,
    entry_id: String,
) -> Result<(), String> {
    let pool = pool_of(&app);
    let (topic_id, meeting_id) = rewrite_source(&pool, &entry_id).await?;
    let topic = require_topic(&pool, &topic_id).await?;
    let inputs = load_extraction_inputs(&pool, &meeting_id).await?;
    let segments = load_segments(&pool, &meeting_id)
        .await
        .map_err(db("Couldn't read the transcript"))?;
    let evidence = for_entry(
        &blocks(&segments),
        &topic_terms(&topic.name, &topic.description),
    );
    // Other entries' decisions only: this entry's own are about to be replaced.
    let mut decisions = ledger::open_decisions(&pool, &topic_id, OPEN_DECISIONS_CAP + 30)
        .await
        .map_err(db("Couldn't load the topic's decisions"))?;
    decisions.retain(|d| d.entry_id != entry_id);
    decisions.truncate(OPEN_DECISIONS_CAP);
    let dir = app.path().app_data_dir().ok();
    let client = reqwest::Client::new();
    let parsed = with_priority(
        Priority::Interactive,
        writer::write_entry(
            &client,
            &inputs.provider_config,
            dir.as_ref(),
            &topic,
            &inputs.summary_markdown,
            inputs.user_notes.as_deref(),
            &evidence,
            &decisions,
        ),
    )
    .await
    .map_err(|e| format!("{e:#}"))?;
    if !ledger::rewrite_entry(&pool, &entry_id, parsed.body)
        .await
        .map_err(db("Couldn't save the rewritten entry"))?
    {
        // The owner edited it while the model ran (the kind never changes).
        return Err(EDITED.to_string());
    }
    emit_updated(&app, Some(&topic_id));
    Ok(())
}

#[tauri::command]
pub async fn api_add_manual_topic_entry<R: Runtime>(
    app: AppHandle<R>,
    topic_id: String,
    body: EntryBody,
) -> Result<String, String> {
    let id = ledger::add_manual_entry(&pool_of(&app), &topic_id, body)
        .await
        .map_err(|e| format!("{e:#}"))?;
    emit_updated(&app, Some(&topic_id));
    Ok(id)
}

#[tauri::command]
pub async fn api_delete_topic_entry<R: Runtime>(
    app: AppHandle<R>,
    entry_id: String,
) -> Result<(), String> {
    let pool = pool_of(&app);
    let topic_id = entry_topic(&pool, &entry_id).await?;
    ledger::delete_entry(&pool, &entry_id)
        .await
        .map_err(db("Couldn't delete the entry"))?;
    emit_updated(&app, Some(&topic_id));
    Ok(())
}

#[tauri::command]
pub async fn api_resolve_topic_suggestion<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    accept: bool,
) -> Result<(), String> {
    let pool = pool_of(&app);
    let result = ledger::resolve_suggestion(&pool, &id, accept).await;
    // A refused accept (e.g. "That entry was deleted.") still marks the suggestion dismissed.
    let out = result.map_err(|e| format!("{e:#}"));
    emit_updated(&app, None);
    out.map(|_| ())
}

#[tauri::command]
pub async fn api_set_action_item_topic<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    topic_id: Option<String>,
) -> Result<(), String> {
    let pool = pool_of(&app);
    let found = Todo::set_topic(&pool, &id, topic_id.as_deref())
        .await
        .map_err(|e| format!("{e:#}"))?;
    if !found {
        return Err("That action item no longer exists".to_string());
    }
    let _ = app.emit(
        ACTION_ITEMS_UPDATED_EVENT,
        serde_json::json!({ "meeting_id": "" }),
    );
    // `null`: a re-tag changes the old topic as well as the new one.
    emit_updated(&app, None);
    Ok(())
}

/// Dismiss completed to-dos from a topic's card (they stay ticked off there until this).
#[tauri::command]
pub async fn api_clear_topic_todos<R: Runtime>(
    app: AppHandle<R>,
    topic_id: String,
    ids: Vec<String>,
) -> Result<u64, String> {
    let n = Todo::clear_done_for_topic(&pool_of(&app), &topic_id, &ids)
        .await
        .map_err(|e| format!("Couldn't dismiss those to-dos: {e}"))?;
    emit_updated(&app, Some(&topic_id));
    Ok(n)
}
