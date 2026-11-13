//! Post-summary topic jobs (specs/0089): the entry writer for meetings linked to a topic and
//! the mention detector for meetings that aren't. Both only ever INSERT (see `ledger.rs`), so
//! re-running after the owner edited or dismissed something changes nothing: an entry exists
//! once per (topic, meeting), and a topic that already has any suggestion or entry from a
//! meeting is never re-suggested from it.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use super::detector::{detect_with_llm, fingerprint, mention_options};
use super::evidence::{blocks, for_entry, for_mentions, load_segments, topic_terms, Block};
use super::ledger::{self, NewSuggestion};
use super::limits::{MENTION_TOPICS_CAP, OPEN_DECISIONS_CAP};
use super::links::topics_for_meeting;
use super::model::Topic;
use super::repo::{emit_updated, TopicsRepository};
use super::writer::{entry_options, write_entry_with_llm};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::llm_activity::{LlmActivityState, Origin, TaskKind};
use crate::summary::provider_config::ProviderConfig;

/// Which of the two prompts a model call serves; they carry different JSON schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobCall {
    Entry,
    Mention,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobReport {
    pub entries: u32,
    pub suggestions: u32,
}

impl JobReport {
    fn add(&mut self, other: JobReport) {
        self.entries += other.entries;
        self.suggestions += other.suggestions;
    }
}

static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Releases a meeting's slot on drop, so an early return never wedges it.
struct Guard(String);

impl Drop for Guard {
    fn drop(&mut self) {
        IN_FLIGHT
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.0);
    }
}

/// `None` means another topic job for this meeting is running; the caller skips.
fn claim(meeting_id: &str) -> Option<Guard> {
    IN_FLIGHT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(meeting_id.to_string())
        .then(|| Guard(meeting_id.to_string()))
}

/// The entry-writer half for every open linked topic without an entry yet.
async fn write_entries<F, Fut>(
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &str,
    notes: Option<&str>,
    blocks: &[Block],
    llm: &F,
) -> Result<JobReport>
where
    F: Fn(JobCall, String, String) -> Fut,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    let mut report = JobReport::default();
    for topic in topics_for_meeting(pool, meeting_id, true).await? {
        if ledger::has_entry(pool, &topic.id, meeting_id).await? {
            continue;
        }
        let evidence = for_entry(blocks, &topic_terms(&topic.name, &topic.description));
        let decisions = ledger::open_decisions(pool, &topic.id, OPEN_DECISIONS_CAP).await?;
        let call = |s: String, u: String| llm(JobCall::Entry, s, u);
        let parsed =
            match write_entry_with_llm(call, &topic, summary, notes, &evidence, &decisions).await {
                Ok(p) => p,
                Err(e) => {
                    log::warn!(
                        "Topic entry for meeting_id {meeting_id} failed (others continue): {e:#}"
                    );
                    continue;
                }
            };
        let inserted =
            ledger::insert_machine_entry(pool, &topic.id, meeting_id, "series", parsed.body).await;
        let Some(_entry_id) = inserted.unwrap_or_else(|e| {
            log::warn!("Couldn't store a topic entry for meeting_id {meeting_id}: {e:#}");
            None
        }) else {
            continue; // closed, deleted or already written while the model ran
        };
        report.entries += 1;
        let rows: Vec<NewSuggestion> = parsed
            .revisions
            .into_iter()
            .map(|(d, note, t)| NewSuggestion {
                topic_id: topic.id.clone(),
                kind: "revision".into(),
                meeting_id: meeting_id.to_string(),
                t_start: t,
                target_entry_id: Some(d.entry_id),
                target_decision_id: Some(d.decision_id),
                text: note,
            })
            .collect();
        if !rows.is_empty() {
            report.suggestions += ledger::insert_suggestions(pool, &rows).await?;
        }
    }
    Ok(report)
}

/// Open topics this meeting isn't linked to, most recently active first, capped.
async fn unlinked_open_topics(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<Topic>> {
    let linked: HashSet<String> = topics_for_meeting(pool, meeting_id, true)
        .await?
        .into_iter()
        .map(|t| t.id)
        .collect();
    let mut open = TopicsRepository::list(pool, Some("open")).await?;
    open.retain(|t| !linked.contains(&t.id));
    open.sort_by(|a, b| b.last_activity.cmp(&a.last_activity)); // stable: name order within ties
    let mut out = Vec::new();
    for s in open.into_iter().take(MENTION_TOPICS_CAP) {
        if let Some(t) = TopicsRepository::get(pool, &s.id).await? {
            out.push(t);
        }
    }
    Ok(out)
}

async fn detect_mentions<F, Fut>(
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &str,
    blocks: &[Block],
    llm: &F,
) -> Result<u32>
where
    F: Fn(JobCall, String, String) -> Fut,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    let topics = unlinked_open_topics(pool, meeting_id).await?;
    if topics.is_empty() {
        return Ok(0);
    }
    let print = fingerprint(summary, &topics);
    let seen: Option<String> =
        sqlx::query_scalar("SELECT fingerprint FROM topic_mention_scans WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await?;
    if seen.as_deref() == Some(print.as_str()) {
        return Ok(0);
    }
    let terms: Vec<String> = topics
        .iter()
        .flat_map(|t| topic_terms(&t.name, &t.description))
        .collect();
    let evidence = for_mentions(blocks, &terms);
    let call = |s: String, u: String| llm(JobCall::Mention, s, u);
    let found = detect_with_llm(call, &topics, summary, &evidence).await?;
    let rows: Vec<NewSuggestion> = found
        .into_iter()
        .map(|(topic, text, t)| NewSuggestion {
            topic_id: topic.id,
            kind: "note".into(),
            meeting_id: meeting_id.to_string(),
            t_start: Some(t),
            target_entry_id: None,
            target_decision_id: None,
            text,
        })
        .collect();
    let stored = ledger::insert_suggestions(pool, &rows).await?;
    sqlx::query(
        "INSERT INTO topic_mention_scans (meeting_id, fingerprint, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(meeting_id) DO UPDATE SET fingerprint = ?2, created_at = ?3",
    )
    .bind(meeting_id)
    .bind(&print)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(stored)
}

async fn run_inner<R, F, Fut>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &str,
    notes: Option<&str>,
    llm: F,
    detect: bool,
) -> Result<JobReport>
where
    R: Runtime,
    F: Fn(JobCall, String, String) -> Fut + Clone,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    let Some(_guard) = claim(meeting_id) else {
        log::info!("Topic job for meeting_id {meeting_id} already running; skipping");
        return Ok(JobReport::default());
    };
    let segments = load_segments(pool, meeting_id).await?;
    let blocks = blocks(&segments);
    if blocks.is_empty() {
        return Ok(JobReport::default());
    }
    let mut report = write_entries(pool, meeting_id, summary, notes, &blocks, &llm).await?;
    if detect {
        match detect_mentions(pool, meeting_id, summary, &blocks, &llm).await {
            Ok(n) => report.suggestions += n,
            Err(e) => log::warn!("Topic mention scan for meeting_id {meeting_id} failed: {e:#}"),
        }
    }
    if report.entries > 0 || report.suggestions > 0 {
        emit_updated(app, None);
    }
    Ok(report)
}

/// Entry writer for linked topics, then the mention detector for the rest. One topic's
/// failure is logged and the others continue.
pub async fn run_for_meeting_with<R, F, Fut>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &str,
    notes: Option<&str>,
    llm: F,
) -> Result<JobReport>
where
    R: Runtime,
    F: Fn(JobCall, String, String) -> Fut + Clone,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    run_inner(app, pool, meeting_id, summary, notes, llm, true).await
}

type LlmFuture =
    std::pin::Pin<Box<dyn Future<Output = std::result::Result<String, String>> + Send>>;

/// The summary provider's model call for either prompt.
fn provider_llm(
    config: &ProviderConfig,
    app_data_dir: Option<std::path::PathBuf>,
) -> impl Fn(JobCall, String, String) -> LlmFuture + Clone {
    let client = reqwest::Client::new();
    let config = config.clone();
    move |call, system, user| {
        let (client, config, dir) = (client.clone(), config.clone(), app_data_dir.clone());
        Box::pin(async move {
            let options = match call {
                JobCall::Entry => entry_options(),
                JobCall::Mention => mention_options(),
            };
            crate::summary::llm_client::generate_summary_with_options(
                &client,
                &config.provider,
                &config.model_name,
                &config.api_key,
                &system,
                &user,
                config.ollama_endpoint.as_deref(),
                config.custom_openai_endpoint.as_deref(),
                None,
                Some(0.0),
                None,
                dir.as_ref(),
                None,
                &options,
            )
            .await
        })
    }
}

/// [`run_for_meeting_with`] over the summary's provider config.
pub async fn run_for_meeting<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &str,
    notes: Option<&str>,
    config: &ProviderConfig,
) -> Result<JobReport> {
    let llm = provider_llm(config, app.path().app_data_dir().ok());
    run_for_meeting_with(app, pool, meeting_id, summary, notes, llm).await
}

/// The background trigger after a summary is stored. Registers a queue row (skipped, with no
/// row at all, when there are no open topics) and never propagates an error.
pub async fn run_background<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &str,
    notes: Option<&str>,
    config: &ProviderConfig,
) {
    match TopicsRepository::list(pool, Some("open")).await {
        Ok(open) if open.is_empty() => return,
        Err(e) => {
            log::warn!("Topic job for meeting_id {meeting_id} couldn't list topics: {e:#}");
            return;
        }
        Ok(_) => {}
    }
    let task = match app.try_state::<LlmActivityState>() {
        Some(state) => {
            let title = match MeetingsRepository::get_meeting_metadata(pool, meeting_id).await {
                Ok(Some(meta)) => format!("Updating topics — {}", meta.title),
                _ => "Updating topics".to_string(),
            };
            Some(Arc::clone(&state.0).start_for(
                TaskKind::Topics,
                Origin::Background,
                title,
                Some(meeting_id.to_string()),
            ))
        }
        None => None,
    };
    let result = run_for_meeting(app, pool, meeting_id, summary, notes, config).await;
    if let Err(e) = &result {
        log::warn!("Topic job for meeting_id {meeting_id} failed (summary unaffected): {e:#}");
    }
    if let Some(t) = task {
        t.finish(result.map(|_| ()).map_err(|e| format!("{e:#}")));
    }
}

/// Entry-writer half only, oldest first, one meeting at a time so each prompt sees the
/// decisions the previous entries recorded. `load` supplies (summary, notes) per meeting; one
/// that can't be loaded is logged and skipped.
pub async fn backfill_with<R, L, LFut, F, Fut>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_ids: &[String],
    load: L,
    llm: F,
) -> JobReport
where
    R: Runtime,
    L: Fn(String) -> LFut,
    LFut: Future<Output = std::result::Result<(String, Option<String>), String>>,
    F: Fn(JobCall, String, String) -> Fut + Clone,
    Fut: Future<Output = std::result::Result<String, String>>,
{
    let mut total = JobReport::default();
    for id in meeting_ids {
        let (summary, notes) = match load(id.clone()).await {
            Ok(v) => v,
            Err(e) => {
                log::info!("Topic backfill skipped meeting_id {id}: {e}");
                continue;
            }
        };
        match run_inner(
            app,
            pool,
            id,
            &summary,
            notes.as_deref(),
            llm.clone(),
            false,
        )
        .await
        {
            Ok(r) => total.add(r),
            Err(e) => log::warn!("Topic backfill for meeting_id {id} failed: {e:#}"),
        }
    }
    total
}

/// Fire-and-forget backfill of a freshly linked topic's earlier meetings.
pub fn spawn_backfill<R: Runtime>(
    app: AppHandle<R>,
    pool: SqlitePool,
    topic_id: String,
    meeting_ids: Vec<String>,
) {
    tauri::async_runtime::spawn(async move {
        let Some(first) = meeting_ids.first() else {
            return;
        };
        let loaded = crate::action_items::commands::load_extraction_inputs(&pool, first).await;
        // The provider config is the same for every meeting; take it from any loadable one.
        let Some(config) = provider_config_for(&pool, &meeting_ids, loaded).await else {
            log::info!("Topic backfill for topic_id {topic_id}: no summary model configured");
            return;
        };
        let llm = provider_llm(&config, app.path().app_data_dir().ok());
        let pool_ref = pool.clone();
        let load = move |id: String| {
            let pool = pool_ref.clone();
            async move {
                crate::action_items::commands::load_extraction_inputs(&pool, &id)
                    .await
                    .map(|i| (i.summary_markdown, i.user_notes))
            }
        };
        let r = backfill_with(&app, &pool, &meeting_ids, load, llm).await;
        log::info!(
            "Topic backfill for topic_id {topic_id}: {} entries, {} suggestions",
            r.entries,
            r.suggestions
        );
        if r.entries > 0 || r.suggestions > 0 {
            emit_updated(&app, Some(&topic_id));
        }
    });
}

async fn provider_config_for(
    pool: &SqlitePool,
    meeting_ids: &[String],
    first: std::result::Result<crate::action_items::commands::ExtractionInputs, String>,
) -> Option<ProviderConfig> {
    if let Ok(i) = first {
        return Some(i.provider_config);
    }
    for id in meeting_ids.iter().skip(1) {
        if let Ok(i) = crate::action_items::commands::load_extraction_inputs(pool, id).await {
            return Some(i.provider_config);
        }
    }
    None
}

/// Tag the meeting's suggested to-dos with its topic when exactly one open topic matches it.
pub async fn pretag_suggestions(pool: &SqlitePool, meeting_id: &str) -> Result<u32> {
    let topics = topics_for_meeting(pool, meeting_id, true).await?;
    let [topic] = topics.as_slice() else {
        return Ok(0);
    };
    let res = sqlx::query(
        "UPDATE action_items SET topic_id = ? WHERE meeting_id = ? AND triage = 'suggested' AND topic_id IS NULL",
    )
    .bind(&topic.id)
    .bind(meeting_id)
    .execute(pool)
    .await
    .context("Couldn't tag the suggested to-dos with their topic")?;
    Ok(res.rows_affected() as u32)
}
