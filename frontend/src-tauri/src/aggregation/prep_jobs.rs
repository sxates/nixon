//! Background pre-generation of pre-call-prep briefs (specs/0036).
//!
//! Keeps briefs for the next ~48h of recurring meetings warm in `meeting_briefs`, so opening
//! a meeting's Prep tab is instant. Mirrors the established background-job pattern
//! (`spawn_retention_sweeper`, `spawn_background_sync`): a fire-and-forget `spawn` with a
//! startup delay + interval loop, single-flighted, whose failures are only logged — it must
//! never block or fail anything else (the specs/0034 extraction contract).
//!
//! Per the owner decision (2026-07-04) briefs pre-generate for ALL recurring meetings
//! regardless of provider (consistent with "provider choice in Settings is the egress
//! consent"). The same [`generate_brief_for_target`] powers the on-demand IPC path.

use crate::aggregation::commands::configured_summary_model;
use crate::aggregation::engine::Stage;
use crate::aggregation::execute_pre_call_prep;
use crate::aggregation::prep_commands::{claim_for_pass, finish_run};
use crate::calendar::eventkit::{self, UpcomingMeeting};
use crate::database::repositories::dismissed_calendar_event::DismissedCalendarEventsRepository;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_brief::{MeetingBrief, MeetingBriefsRepository};
use crate::llm_activity::{LlmActivityState, Origin, QueuedHandle, TaskKind};
use crate::state::AppState;
use crate::summary::processor::generate_summary_with_retry;
use crate::summary::provider_config::resolve_provider_config;
use crate::summary::resolve_context_budget;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// First pass shortly after launch, then a periodic refresh. Kept modest — the fingerprint
/// guard means a pass usually generates nothing.
const STARTUP_DELAY: Duration = Duration::from_secs(90);
const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// "today + tomorrow" — enough for morning prep and next-day lookahead.
const HORIZON_HOURS: u32 = 48;
/// At most this many prior occurrences feed a brief (spec: "previous 2").
const MAX_PRIOR: i64 = 2;

/// Consecutive failed passes before the background generator stops retrying a brief
/// (specs/0052). Bounds a permanently-broken brief at ~45 min of GPU instead of forever,
/// while still self-healing from a transient failure (Ollama restarting, model swapping).
pub(crate) const MAX_BRIEF_FAILURES: i64 = 3;

/// Whether the background pass should skip this brief entirely.
///
/// Only skips a brief that has failed `MAX_BRIEF_FAILURES` times against the SAME input: a
/// fingerprint change means new input, which always earns a fresh set of attempts.
pub(crate) fn should_skip_brief(
    existing: Option<&MeetingBrief>,
    current_fingerprint: &str,
) -> bool {
    let Some(existing) = existing else {
        return false;
    };
    existing.failure_count >= MAX_BRIEF_FAILURES
        && existing.source_fingerprint.as_deref() == Some(current_fingerprint)
}

/// Single-flight guard so overlapping triggers (startup, interval, on-demand focus) don't run
/// concurrent passes.
static PASS_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Tells Today / an open Prep tab to re-read briefs.
pub(crate) const PREP_BRIEFS_UPDATED: &str = "prep-briefs-updated";

/// Spawn the background prep generator (call once at startup, like the retention sweeper).
pub fn spawn_prep_generator<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        info!(
            "prep generator started (first run in {:?}, then every {:?})",
            STARTUP_DELAY, REFRESH_INTERVAL
        );
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            run_prep_pass(&app).await;
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}

/// One pass: for every upcoming recurring occurrence in the horizon that has prior content,
/// ensure a scheduled prep row exists and its brief is generated + up to date. Best-effort;
/// never propagates errors.
///
/// specs/0074 W3: plan first, then execute. Every brief that needs generating is queued
/// before the first one starts, so the queue shows the whole pass as *Waiting* rows; briefs
/// that are up to date register nothing at all.
pub async fn run_prep_pass<R: Runtime>(app: &AppHandle<R>) {
    let _guard = match PASS_LOCK.try_lock() {
        Ok(g) => g,
        Err(_) => return, // a pass is already running
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let pool = state.db_manager.pool().clone();

    // No summary provider configured → nothing can be generated. Skip the whole pass with a
    // single log rather than attempting (and erroring on) every event (code-review A3).
    if configured_summary_model(&pool).await.is_err() {
        info!("prep: no summary model configured; skipping background generation pass");
        return;
    }

    let upcoming = upcoming_for_horizon(app, HORIZON_HOURS).await;
    if upcoming.is_empty() {
        info!("prep: pass complete — no upcoming meetings in the next {HORIZON_HOURS}h");
        return;
    }
    let (planned, plan_failed) = plan_pass(app, &pool, &upcoming).await;
    let queued = planned.len();
    let failed = plan_failed + execute_pass(app, &pool, planned).await;

    // Always log the pass outcome, so a healthy generator, a misconfigured one, and one
    // skipping every event can be told apart from outside.
    info!(
        "prep: pass complete — {} upcoming meeting(s) considered, {queued} queued, {failed} failed",
        upcoming.len()
    );
}

/// A brief the pass will generate, with its waiting row (`None` when there is no registry).
pub(crate) type PlannedBrief = (String, Option<QueuedHandle>);

/// Plan half of the pass: queue every brief that needs generating. Returns the queue and the
/// number of events that could not be planned.
pub(crate) async fn plan_pass<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    upcoming: &[UpcomingMeeting],
) -> (Vec<PlannedBrief>, usize) {
    let (mut planned, mut failed, mut changed) = (Vec::new(), 0usize, false);
    for ev in upcoming {
        let (target_id, plan) = match plan_event(pool, ev).await {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => {
                failed += 1;
                warn!(
                    "prep: could not plan event {:?} (continuing): {:#}",
                    ev.id, e
                );
                continue;
            }
        };
        match plan {
            // A manual run already holds it; it will finish the job and say so.
            BriefPlan::Generate { .. }
                if crate::aggregation::prep_commands::is_generating(&target_id) => {}
            BriefPlan::Generate { .. } => match enqueue_brief(app, pool, &target_id).await {
                Slot::Queued(handle) => planned.push((target_id, handle)),
                Slot::Busy => {}
            },
            BriefPlan::NoPriors => {
                changed |= record_no_priors(pool, &target_id).await.unwrap_or(false)
            }
            BriefPlan::UpToDate | BriefPlan::GivenUp => {}
        }
    }
    if changed {
        briefs_updated(app);
    }
    (planned, failed)
}

/// Execute half: generate each queued brief in order. Emits `prep-briefs-updated` after each
/// brief whose status may have changed, so Today and an open Prep tab refresh as they land
/// rather than when the whole pass ends. Returns the number that failed.
pub(crate) async fn execute_pass<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    planned: Vec<PlannedBrief>,
) -> usize {
    let mut failed = 0usize;
    for (target_id, handle) in planned {
        // Claimed like a manual run, so a Regenerate or link supersedes it; `None` when a
        // click took the brief over while it waited.
        let Some((run_id, cancel)) = claim_for_pass(&target_id, handle.as_ref()) else {
            continue;
        };
        let result =
            generate_brief_for_target(app, pool, &target_id, false, &cancel, |_| {}, handle).await;
        finish_run(&target_id, &run_id);
        if cancel.is_cancelled() {
            continue; // superseded; the replacement run reports
        }
        if !matches!(result, Ok(false)) {
            briefs_updated(app);
        }
        if let Err(e) = result {
            failed += 1;
            warn!("prep: brief for {target_id} failed (continuing): {e:#}");
        }
    }
    failed
}

fn briefs_updated<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit(PREP_BRIEFS_UPDATED, ());
}

/// For one upcoming calendar occurrence: if it's recurring (has ≥1 prior occurrence with
/// content), mint/return its scheduled prep row and plan its brief. A non-recurring
/// occurrence is skipped here — its scheduled row is minted lazily when the user opens prep
/// (`api_ensure_scheduled_meeting`).
async fn plan_event(
    pool: &SqlitePool,
    ev: &UpcomingMeeting,
) -> anyhow::Result<Option<(String, BriefPlan)>> {
    let Some(occurrence_start) = parse_start(&ev.starts_at) else {
        return Ok(None);
    };
    let series_key = ev.external_id.as_deref();
    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        series_key,
        &ev.title,
        occurrence_start,
        MAX_PRIOR,
    )
    .await?;
    if prior.is_empty() {
        return Ok(None); // not recurring / no prior content → no brief
    }

    let target_id = MeetingsRepository::upsert_scheduled_meeting(
        pool,
        &ev.id,
        series_key,
        &ev.title,
        occurrence_start,
    )
    .await?
    .into_id();
    let plan = plan_brief(pool, &target_id, false).await?;
    Ok(Some((target_id, plan)))
}

/// Whether a manual trigger found a waiting row for its brief.
pub(crate) enum Slot {
    /// Queued (`None`: no registry to show it in, e.g. tests) — go ahead.
    Queued(Option<QueuedHandle>),
    /// Already queued or running elsewhere — that run will do the work.
    Busy,
}

/// Register the *Waiting* row for one brief (specs/0074 W3).
pub(crate) async fn enqueue_brief<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    target_meeting_id: &str,
) -> Slot {
    let Some(registry) = app
        .try_state::<LlmActivityState>()
        .map(|s| Arc::clone(&s.0))
    else {
        return Slot::Queued(None);
    };
    // Queue before the first await, so nothing separates a caller's PREP_RUNS claim from
    // its row (a Regenerate's cancel_run then always finds both); name it afterwards.
    let meeting = Some(target_meeting_id.to_string());
    let generic = "Preparing brief";
    let Some(handle) =
        registry.enqueue_for(TaskKind::PrepBrief, Origin::Background, generic, meeting)
    else {
        return Slot::Busy;
    };
    handle.relabel(brief_task_label(pool, target_meeting_id).await);
    Slot::Queued(Some(handle))
}

/// What a brief needs, decided before any task starts (specs/0074 W3) — so a pass over
/// up-to-date briefs records nothing and cannot flood the 20-entry history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BriefPlan {
    UpToDate,
    NoPriors,
    /// Failed [`MAX_BRIEF_FAILURES`] times against unchanged input (specs/0052).
    GivenUp,
    Generate {
        prior: Vec<String>,
        fingerprint: String,
        /// The input changed since the last attempt, so the failure count restarts.
        input_changed: bool,
    },
}

/// The decision half of generation: reads only, writes nothing.
pub(crate) async fn plan_brief(
    pool: &SqlitePool,
    target_meeting_id: &str,
    force: bool,
) -> anyhow::Result<BriefPlan> {
    let meta = MeetingsRepository::get_meeting_metadata(pool, target_meeting_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("target meeting {target_meeting_id} not found"))?;

    // EFFECTIVE key — calendar-stamped, else the manual meeting_series_links key
    // (specs/0041 WS4) — so a manually associated series briefs like a calendar one.
    let series_key =
        MeetingsRepository::resolve_effective_series_key(pool, target_meeting_id).await?;
    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        series_key.as_deref(),
        &meta.title,
        meta.created_at.0,
        MAX_PRIOR,
    )
    .await?;
    if prior.is_empty() {
        return Ok(BriefPlan::NoPriors);
    }

    let fingerprint = source_fingerprint(pool, &prior).await;
    let existing = MeetingBriefsRepository::get(pool, target_meeting_id).await?;
    if !force {
        if existing.as_ref().is_some_and(|e| {
            e.status == "ready" && e.source_fingerprint.as_deref() == Some(fingerprint.as_str())
        }) {
            return Ok(BriefPlan::UpToDate);
        }
        // specs/0052: stop burning GPU on a brief that has failed repeatedly against input
        // that has not changed. Resumes on a fingerprint change or a manual retry.
        if should_skip_brief(existing.as_ref(), &fingerprint) {
            info!(
                "prep: skipping {} — {} consecutive failures against unchanged input",
                target_meeting_id, MAX_BRIEF_FAILURES
            );
            return Ok(BriefPlan::GivenUp);
        }
    }
    let input_changed = existing
        .as_ref()
        .is_some_and(|e| e.source_fingerprint.as_deref() != Some(fingerprint.as_str()));
    Ok(BriefPlan::Generate {
        prior,
        fingerprint,
        input_changed,
    })
}

/// Stable 'none' fingerprint so it isn't recomputed every pass until a prior appears.
/// Returns whether the status changed.
async fn record_no_priors(pool: &SqlitePool, target_meeting_id: &str) -> anyhow::Result<bool> {
    let was = MeetingBriefsRepository::get(pool, target_meeting_id)
        .await?
        .map(|b| b.status);
    MeetingBriefsRepository::upsert_status(pool, target_meeting_id, "none", Some("none")).await?;
    Ok(was.as_deref() != Some("none"))
}

/// Generate + cache the prep brief for a target meeting (the upcoming/scheduled occurrence),
/// unless a `ready` brief with the same input fingerprint already exists (`force` overrides).
///
/// Shared by the background pass and the on-demand IPC. `queued` is the caller's *Waiting*
/// row: it starts only when the plan says to generate, and otherwise disappears without a
/// trace. Returns whether the brief's status changed. Emits stage progress via `on_progress`
/// (the IPC forwards it to `prep-brief-*` events; the background pass passes a no-op).
/// - No prior occurrences → records `status = 'none'` (nothing to brief).
/// - No summary provider configured → returns an error (the caller surfaces it).
/// - Generation error → records `status = 'failed'` WITH the input fingerprint and bumps
///   `failure_count`. It retries on the next pass, but after [`MAX_BRIEF_FAILURES`]
///   consecutive failures against that same fingerprint the pass skips it entirely
///   (specs/0052) until the input changes or the user retries.
pub async fn generate_brief_for_target<R: Runtime, P: Fn(Stage)>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    target_meeting_id: &str,
    force: bool,
    cancel: &CancellationToken,
    on_progress: P,
    queued: Option<QueuedHandle>,
) -> anyhow::Result<bool> {
    let plan = match plan_brief(pool, target_meeting_id, force).await {
        Ok(plan) => plan,
        Err(e) => {
            if let Some(q) = queued {
                q.start().finish(Err(format!("{e:#}")));
            }
            return Err(e);
        }
    };
    let (prior, fingerprint, input_changed) = match plan {
        BriefPlan::Generate {
            prior,
            fingerprint,
            input_changed,
        } => (prior, fingerprint, input_changed),
        BriefPlan::NoPriors => return record_no_priors(pool, target_meeting_id).await,
        BriefPlan::UpToDate | BriefPlan::GivenUp => return Ok(false),
    };

    // specs/0052: registered as background work so the queue shows it.
    let task = queued.map(QueuedHandle::start);
    let forward = |stage: Stage| {
        if let Some(t) = task.as_ref() {
            t.progress(stage_note(stage));
        }
        on_progress(stage);
    };
    let result = generate_brief_inner(
        app,
        pool,
        target_meeting_id,
        (prior, fingerprint, input_changed),
        cancel,
        forward,
    )
    .await;

    if let Some(t) = task {
        t.finish(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
    }
    result.map(|()| true)
}

/// Human-readable note for the activity row, mirroring the engine's stages.
fn stage_note(stage: Stage) -> String {
    match stage {
        Stage::Gathering => "gathering prior meetings".to_string(),
        Stage::Mapping { current, total } => format!("reading meeting {current} of {total}"),
        Stage::Reducing => "composing the brief".to_string(),
    }
}

/// Label for the activity row — the meeting title is what the user recognizes. Best-effort:
/// a failed lookup falls back to a generic label rather than stopping generation.
async fn brief_task_label(pool: &SqlitePool, target_meeting_id: &str) -> String {
    match MeetingsRepository::get_meeting_metadata(pool, target_meeting_id).await {
        Ok(Some(meta)) => format!("Preparing brief — {}", meta.title),
        _ => "Preparing brief".to_string(),
    }
}

async fn generate_brief_inner<R: Runtime, P: Fn(Stage)>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    target_meeting_id: &str,
    (prior, fingerprint, input_changed): (Vec<String>, String, bool),
    cancel: &CancellationToken,
    on_progress: P,
) -> anyhow::Result<()> {
    // New input => fresh attempts. Must happen before this pass can record a failure of its
    // own, so the counter always describes consecutive failures against ONE fingerprint.
    if input_changed {
        MeetingBriefsRepository::reset_failures(pool, target_meeting_id).await?;
    }

    // Resolve the provider up-front. If none/misconfigured, return an Err (do NOT leave the row
    // 'pending' — that would strand the on-demand UI on a permanent spinner; the caller surfaces
    // the error and the background pass already skips when no provider is configured). code-review A3.
    let (provider_name, model) = match configured_summary_model(pool).await {
        Ok(pm) => pm,
        Err(e) => {
            return Err(e.context(
                "No summary model is configured — choose a provider and model in Settings to generate a prep brief",
            ));
        }
    };
    let provider_config = match resolve_provider_config(pool, &provider_name, &model).await {
        Ok(c) => c,
        Err(e) => {
            return Err(e.context("Could not resolve the summary provider for the prep brief"));
        }
    };

    MeetingBriefsRepository::upsert_status(pool, target_meeting_id, "pending", None).await?;
    info!(
        "prep: generating brief for {} from {} prior occurrence(s) (provider {:?})",
        target_meeting_id,
        prior.len(),
        provider_config.provider
    );

    let budget_tokens = resolve_context_budget(
        &provider_config.provider,
        &provider_config.model_name,
        provider_config.ollama_endpoint.as_deref(),
    )
    .await;

    let app_data_dir = app.path().app_data_dir().ok();
    let client = reqwest::Client::new();
    let llm = move |system: String, user: String| {
        let cfg = provider_config.clone();
        let client = client.clone();
        let cancel = cancel.clone();
        let app_data_dir = app_data_dir.clone();
        async move {
            generate_summary_with_retry(
                &client,
                &cfg.provider,
                &cfg.model_name,
                &cfg.api_key,
                &system,
                &user,
                cfg.ollama_endpoint.as_deref(),
                cfg.custom_openai_endpoint.as_deref(),
                // specs/0052: a prep brief synthesizes 2 prior summaries; p90 of healthy
                // runs was ~1,800 tokens. A user-configured CustomOpenAI value still wins.
                cfg.custom_openai_max_tokens
                    .or(Some(crate::summary::llm_client::PREP_BRIEF_MAX_TOKENS)),
                cfg.custom_openai_temperature,
                cfg.custom_openai_top_p,
                app_data_dir.as_ref(),
                Some(&cancel),
            )
            .await
        }
    };

    let outcome =
        execute_pre_call_prep(pool, &prior, llm, budget_tokens, cancel, on_progress).await;
    if cancel.is_cancelled() {
        // Superseded by a Regenerate or a series link: the replacement writes the brief. A
        // stale answer or a "failed" here would overwrite or penalise it.
        return Err(anyhow::anyhow!("prep brief generation was superseded"));
    }
    match outcome {
        Ok(answer) => {
            let sources_json =
                serde_json::to_string(&answer.sources).unwrap_or_else(|_| "[]".into());
            MeetingBriefsRepository::upsert_ready(
                pool,
                target_meeting_id,
                &answer.markdown,
                &sources_json,
                &fingerprint,
                &provider_name,
                &model,
            )
            .await?;
            info!("prep: brief ready for {}", target_meeting_id);
            Ok(())
        }
        Err(e) => {
            // Retried on the next pass (self-heals transient errors), but bounded now: after
            // MAX_BRIEF_FAILURES consecutive failures against unchanged input the pass skips
            // it entirely (specs/0052).
            //
            // The fingerprint IS recorded on failure — without it `should_skip_brief` could
            // never match and the give-up rule would never bind.
            MeetingBriefsRepository::upsert_status(
                pool,
                target_meeting_id,
                "failed",
                Some(fingerprint.as_str()),
            )
            .await?;
            let count = MeetingBriefsRepository::record_failure(pool, target_meeting_id).await?;
            warn!(
                "prep: brief for {} failed ({}/{} consecutive)",
                target_meeting_id, count, MAX_BRIEF_FAILURES
            );
            Err(e)
        }
    }
}

/// Fingerprint of the generation INPUT: the ordered prior occurrence ids plus each one's
/// summary version stamp. Changes when a prior summary is regenerated or a newer occurrence
/// enters the set, invalidating the cached brief.
async fn source_fingerprint(pool: &SqlitePool, prior_ids: &[String]) -> String {
    let mut parts = Vec::with_capacity(prior_ids.len());
    for id in prior_ids {
        let stamp: Option<String> =
            sqlx::query_scalar("SELECT updated_at FROM summary_processes WHERE meeting_id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        parts.push(format!("{id}:{}", stamp.unwrap_or_default()));
    }
    crate::action_items::diff::stable_text_fingerprint(&parts.join("|"))
}

/// Upcoming occurrences for the horizon, from whichever calendar source is active — the same
/// active-source logic as `api_get_upcoming_meetings`.
///
/// Events the user has hidden from their agenda (specs/0026) are dropped. This was missing
/// until 2026-09-21 and was the expensive half of that bug: every hidden event on the
/// horizon got a prep brief generated for it, which is a real LLM call per event per pass.
/// Hiding "Lunch" was buying a summarization of lunch.
async fn upcoming_for_horizon<R: Runtime>(app: &AppHandle<R>, hours: u32) -> Vec<UpcomingMeeting> {
    let events = if crate::calendar::google_is_active_source(app).await {
        use crate::calendar::google::sync::{sync_if_stale, SyncTrigger};
        sync_if_stale(app, SyncTrigger::Prep).await;
        let now = Utc::now();
        let end = now + chrono::Duration::hours(i64::from(hours));
        match crate::calendar::google::sync::db_pool(app) {
            Some(pool) => {
                crate::calendar::google::sync::cached_upcoming_between(&pool, now, end).await
            }
            None => Vec::new(),
        }
    } else {
        tokio::task::spawn_blocking(move || eventkit::upcoming_meetings(hours))
            .await
            .unwrap_or_default()
    };

    // A pool we can't read means "prep everything" — the same fail-open choice
    // `api_get_upcoming_meetings` makes, and here it only costs tokens, never a missed
    // meeting.
    let Some(pool) = crate::calendar::google::sync::db_pool(app) else {
        return events;
    };
    let dismissed = match DismissedCalendarEventsRepository::all(&pool).await {
        Ok(set) => set,
        Err(e) => {
            log::warn!("prep: could not read hidden events ({e}); prepping all");
            return events;
        }
    };
    if dismissed.is_empty() {
        return events;
    }
    let before = events.len();
    let kept: Vec<UpcomingMeeting> = events
        .into_iter()
        .filter(|e| !crate::calendar::day_agenda::is_event_dismissed(e, &dismissed))
        .collect();
    if kept.len() != before {
        log::info!(
            "prep: skipping {} hidden event(s) on the horizon",
            before - kept.len()
        );
    }
    kept
}

fn parse_start(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod give_up_tests {
    use super::*;

    fn brief(status: &str, fingerprint: Option<&str>, failure_count: i64) -> MeetingBrief {
        MeetingBrief {
            meeting_id: "m1".into(),
            status: status.into(),
            brief_markdown: None,
            sources_json: None,
            source_fingerprint: fingerprint.map(|s| s.to_string()),
            model_provider: None,
            model_name: None,
            generated_at: None,
            created_at: "2026-08-18T00:00:00Z".into(),
            updated_at: "2026-08-18T00:00:00Z".into(),
            failure_count,
        }
    }

    #[test]
    fn keeps_retrying_below_the_cap() {
        assert!(!should_skip_brief(
            Some(&brief("failed", Some("fp-1"), 2)),
            "fp-1"
        ));
    }

    #[test]
    fn gives_up_at_the_cap() {
        assert!(should_skip_brief(
            Some(&brief("failed", Some("fp-1"), 3)),
            "fp-1"
        ));
    }

    /// New input always earns a fresh set of attempts.
    #[test]
    fn a_fingerprint_change_resumes_a_given_up_brief() {
        assert!(!should_skip_brief(
            Some(&brief("failed", Some("fp-1"), 9)),
            "fp-2"
        ));
    }

    #[test]
    fn a_brief_with_no_row_is_never_skipped() {
        assert!(!should_skip_brief(None, "fp-1"));
    }

    /// A 'failed' row with no fingerprint recorded (the pre-0052 shape) must not be
    /// treated as matching the current fingerprint.
    #[test]
    fn a_null_fingerprint_never_matches() {
        assert!(!should_skip_brief(Some(&brief("failed", None, 9)), "fp-1"));
    }
}

#[cfg(test)]
mod instrumentation_tests {
    use super::*;
    use crate::llm_activity::LlmTaskRegistry;

    /// The prep generator must appear as BACKGROUND work so the sidebar surfaces it — the
    /// whole reason specs/0052 exists is that this work was invisible.
    #[test]
    fn a_prep_task_surfaces_as_background_work() {
        let registry = Arc::new(LlmTaskRegistry::new());
        let task = registry.start_for(
            TaskKind::PrepBrief,
            Origin::Background,
            "Preparing brief — Weekly 1:1",
            Some("m1".into()),
        );
        task.progress(stage_note(Stage::Mapping {
            current: 1,
            total: 2,
        }));

        let view = registry.view();
        assert_eq!(view.running.len(), 1);
        assert_eq!(view.running[0].label, "Preparing brief — Weekly 1:1");
        assert_eq!(
            view.running[0].note.as_deref(),
            Some("reading meeting 1 of 2")
        );

        task.finish(Err("No summary model is configured".into()));
        let view = registry.view();
        assert!(view.running.is_empty());
        assert!(view.has_failure);
        // Retry needs the meeting id to address the right meeting_briefs row.
        assert_eq!(view.history[0].meeting_id.as_deref(), Some("m1"));
    }

    #[test]
    fn stage_notes_are_human_readable() {
        assert_eq!(stage_note(Stage::Gathering), "gathering prior meetings");
        assert_eq!(
            stage_note(Stage::Mapping {
                current: 2,
                total: 5
            }),
            "reading meeting 2 of 5"
        );
        assert_eq!(stage_note(Stage::Reducing), "composing the brief");
    }
}

#[cfg(test)]
#[path = "prep_jobs_tests.rs"]
mod queue_tests;
