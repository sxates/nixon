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
use crate::calendar::eventkit::{self, UpcomingMeeting};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_brief::{MeetingBrief, MeetingBriefsRepository};
use crate::llm_activity::{LlmActivityState, Origin, TaskKind};
use crate::state::AppState;
use crate::summary::processor::generate_summary_with_retry;
use crate::summary::provider_config::resolve_provider_config;
use crate::summary::resolve_context_budget;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};
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
    let considered = upcoming.len();
    let mut failed = 0usize;
    let cancel = CancellationToken::new();
    for ev in upcoming {
        if let Err(e) = ensure_brief_for_event(app, &pool, &ev, &cancel).await {
            failed += 1;
            warn!(
                "prep: brief for event {:?} failed (continuing): {:#}",
                ev.id, e
            );
        }
    }

    // Always log the pass outcome. Every other exit from this function used to be silent —
    // including the common one where no upcoming meeting is recurring-with-priors — so a
    // healthy generator, a misconfigured one, and one skipping every event were
    // indistinguishable from outside. That is the same "I can't tell what it's doing"
    // problem specs/0052 exists to fix, and the sidebar can't help here because the
    // activity registry is only entered once `generate_brief_for_target` starts.
    info!("prep: pass complete — {considered} upcoming meeting(s) considered, {failed} failed");
}

/// For one upcoming calendar occurrence: if it's recurring (has ≥1 prior occurrence with
/// content), mint/return its scheduled prep row and (re)generate the brief when stale. A
/// non-recurring occurrence is skipped here — its scheduled row is minted lazily when the
/// user opens prep (`api_ensure_scheduled_meeting`).
async fn ensure_brief_for_event<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    ev: &UpcomingMeeting,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let Some(occurrence_start) = parse_start(&ev.starts_at) else {
        return Ok(());
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
        return Ok(()); // not recurring / no prior content → no brief
    }

    let target_id = MeetingsRepository::upsert_scheduled_meeting(
        pool,
        &ev.id,
        series_key,
        &ev.title,
        occurrence_start,
    )
    .await?;

    generate_brief_for_target(app, pool, &target_id, false, cancel, |_| {}).await
}

/// Generate + cache the prep brief for a target meeting (the upcoming/scheduled occurrence),
/// unless a `ready` brief with the same input fingerprint already exists (`force` overrides).
///
/// Shared by the background pass and the on-demand IPC. Emits stage progress via `on_progress`
/// (the IPC forwards it to `prep-brief-*` events; the background pass passes a no-op).
/// - No prior occurrences → records `status = 'none'` (nothing to brief).
/// - No summary provider configured → leaves the row `pending` and returns Ok (retried later).
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
) -> anyhow::Result<()> {
    // specs/0052: register this as background work so the sidebar can show it. Prep runs
    // every 30 minutes, including mid-meeting, and used to be completely invisible.
    let registry = app
        .try_state::<LlmActivityState>()
        .map(|state| Arc::clone(&state.0));
    let task = match registry {
        Some(registry) => {
            let label = brief_task_label(pool, target_meeting_id).await;
            Some(registry.start_for(
                TaskKind::PrepBrief,
                Origin::Background,
                label,
                Some(target_meeting_id.to_string()),
            ))
        }
        None => None,
    };

    let forward = |stage: Stage| {
        if let Some(t) = task.as_ref() {
            t.progress(stage_note(stage));
        }
        on_progress(stage);
    };

    let result = generate_brief_inner(app, pool, target_meeting_id, force, cancel, forward).await;

    if let Some(t) = task {
        t.finish(result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
    }
    result
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
    force: bool,
    cancel: &CancellationToken,
    on_progress: P,
) -> anyhow::Result<()> {
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
        // Stable fingerprint so 'none' isn't recomputed every pass until a prior appears.
        MeetingBriefsRepository::upsert_status(pool, target_meeting_id, "none", Some("none"))
            .await?;
        return Ok(());
    }

    let fingerprint = source_fingerprint(pool, &prior).await;
    let existing = MeetingBriefsRepository::get(pool, target_meeting_id).await?;

    if !force {
        if let Some(existing) = existing.as_ref() {
            if existing.status == "ready"
                && existing.source_fingerprint.as_deref() == Some(fingerprint.as_str())
            {
                return Ok(()); // up to date
            }
        }
        // specs/0052: stop burning GPU on a brief that has failed repeatedly against input
        // that has not changed. Resumes on a fingerprint change or a manual retry.
        if should_skip_brief(existing.as_ref(), &fingerprint) {
            info!(
                "prep: skipping {} — {} consecutive failures against unchanged input",
                target_meeting_id, MAX_BRIEF_FAILURES
            );
            return Ok(());
        }
    }

    // New input => fresh attempts. Must happen before this pass can record a failure of its
    // own, so the counter always describes consecutive failures against ONE fingerprint.
    if existing
        .as_ref()
        .is_some_and(|e| e.source_fingerprint.as_deref() != Some(fingerprint.as_str()))
    {
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

    match execute_pre_call_prep(pool, &prior, llm, budget_tokens, cancel, on_progress).await {
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
async fn upcoming_for_horizon<R: Runtime>(app: &AppHandle<R>, hours: u32) -> Vec<UpcomingMeeting> {
    if crate::calendar::google_is_active_source(app).await {
        crate::calendar::google::sync::sync_if_stale(app).await;
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
    }
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
