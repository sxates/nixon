//! The app-facing lifecycle hooks (specs/0072 §"Where the hooks live"): each is the one line
//! a processing step calls when it finishes. They write through `state.rs`, emit
//! `meeting-audio-state-changed` for every `audio_state` write, and hand the meeting to the
//! executor. All best-effort: a lifecycle failure is logged and never fails the step that
//! called it.

use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::policy::{AudioRetention, AudioState};
use super::state::{self, scan_folder, DiarizationGate};
use super::sweep::{self, ApplyOutcome, RetentionReport, SweepOptions};
use crate::state::AppState;

/// Emitted on every `audio_state` write so an open meeting page updates without a refetch.
pub const EVENT_AUDIO_STATE_CHANGED: &str = "meeting-audio-state-changed";

/// Payload of [`EVENT_AUDIO_STATE_CHANGED`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStateChanged {
    pub meeting_id: String,
    pub state: AudioState,
}

/// A post-processing hook that finds the folder busy retries this often…
const HOOK_RETRY_DELAY: Duration = Duration::from_secs(60);
/// …this many times, then leaves the meeting to the hourly sweep.
const HOOK_RETRIES: usize = 5;
/// First scheduled sweep after launch, then every hour.
const FIRST_SWEEP_DELAY: Duration = Duration::from_secs(60);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// The startup reconcile waits at most this long for one diarization pass.
const RECONCILE_DIARIZATION_WAIT: Duration = Duration::from_secs(60 * 60);

pub(crate) fn pool<R: Runtime>(app: &AppHandle<R>) -> Option<SqlitePool> {
    app.try_state::<AppState>()
        .map(|s| s.db_manager.pool().clone())
}

pub(crate) fn emit_state<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, state: AudioState) {
    let _ = app.emit(
        EVENT_AUDIO_STATE_CHANGED,
        AudioStateChanged {
            meeting_id: meeting_id.to_string(),
            state,
        },
    );
}

/// The diarization setting + models-present half of the "applies" test.
pub async fn diarization_gate() -> DiarizationGate {
    DiarizationGate {
        enabled: crate::diarization::settings::load_settings()
            .await
            .diarization_enabled,
        models_present: crate::diarization::models::models_present(),
    }
}

/// The saved retention policy. `None` (and a warning) when preferences can't be read: the
/// executor then does nothing rather than guess.
pub(crate) async fn saved_policy<R: Runtime>(app: &AppHandle<R>) -> Option<AudioRetention> {
    match crate::audio::recording_preferences::load_recording_preferences(app).await {
        Ok(prefs) => Some(prefs.effective_audio_retention()),
        Err(e) => {
            log::warn!("Audio lifecycle: could not read the retention setting: {e:#}");
            None
        }
    }
}

/// Options for a scheduled pass (and the post-processing hook): compress at most one
/// meeting, never while recording, only when compression is switched on.
pub(crate) async fn scheduled_options() -> SweepOptions {
    SweepOptions::scheduled(
        crate::audio::recording_commands::is_recording().await,
        crate::audio::ffmpeg::find_ffmpeg_path(),
    )
}

/// Apply the saved policy to one meeting in the background, retrying while its folder is
/// busy (the hook usually fires while the job that finished still holds the lease).
pub fn spawn_apply<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    let (app, meeting_id) = (app.clone(), meeting_id.to_string());
    tauri::async_runtime::spawn(async move {
        for attempt in 0..HOOK_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(HOOK_RETRY_DELAY).await;
            }
            let (Some(pool), Some(policy)) = (pool(&app), saved_policy(&app).await) else {
                return;
            };
            let opts = scheduled_options().await;
            let now = chrono::Utc::now();
            match sweep::apply_one(&pool, &meeting_id, policy, now, &opts).await {
                Ok(ApplyOutcome::Busy { .. }) => continue,
                Ok(ApplyOutcome::Purged(_)) => emit_state(&app, &meeting_id, AudioState::Purged),
                Ok(_) => {}
                Err(e) => log::warn!("Audio lifecycle: meeting {meeting_id}: {e:#}"),
            }
            return;
        }
        log::info!(
            "Audio lifecycle: meeting {meeting_id} stayed busy; the hourly sweep will handle it"
        );
    });
}

/// Reevaluate one meeting; when it became processed, emit and hand it to the executor.
async fn reevaluate_and_apply<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    transcribed: bool,
) -> Option<AudioState> {
    let pool = pool(app)?;
    match state::reevaluate(&pool, meeting_id, diarization_gate().await, transcribed).await {
        Ok(Some(new_state)) => {
            emit_state(app, meeting_id, new_state);
            spawn_apply(app, meeting_id);
            Some(new_state)
        }
        Ok(None) => None,
        Err(e) => {
            log::warn!("Audio lifecycle: {e:#}");
            None
        }
    }
}

/// What [`finish_processing`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishOutcome {
    /// Speaker identification was started; its completion hook reevaluates.
    Diarizing,
    /// Reevaluated now; `Some` when the state changed.
    Evaluated(Option<AudioState>),
}

/// Diarize the meeting if that applies and hasn't happened, otherwise reevaluate it now.
/// Idempotent. `transcribed` is true when the caller just finished a transcription pass
/// (import).
pub async fn finish_processing<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    transcribed: bool,
) -> FinishOutcome {
    if let Some(pool) = pool(app) {
        if let Ok(Some(row)) = state::load_row(&pool, meeting_id).await {
            let unidentified =
                row.state() == AudioState::Pending && row.speakers_identified_at.is_none();
            if let Some(folder) = row.folder().filter(|_| unidentified) {
                let audio = scan_folder(&folder);
                let finalized = audio.as_ref().is_some_and(|a| !a.recording);
                let has_system = audio.is_some_and(|a| a.system_channel);
                if finalized && diarization_gate().await.applies(has_system) {
                    crate::diarization::launch::diarize_meeting(app.clone(), meeting_id.into());
                    return FinishOutcome::Diarizing;
                }
            }
        }
    }
    FinishOutcome::Evaluated(reevaluate_and_apply(app, meeting_id, transcribed).await)
}

/// Diarization reached its terminal outcome (`diarization/launch.rs`).
pub async fn on_diarization_outcome<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, ok: bool) {
    let Some(pool) = pool(app) else { return };
    if ok {
        if let Err(e) = state::mark_speakers_identified(&pool, meeting_id).await {
            log::warn!("Audio lifecycle: {e:#}");
        }
        reevaluate_and_apply(app, meeting_id, false).await;
        return;
    }
    match state::mark_failed_if_pending(&pool, meeting_id).await {
        Ok(true) => emit_state(app, meeting_id, AudioState::Failed),
        Ok(false) => {}
        Err(e) => log::warn!("Audio lifecycle: {e:#}"),
    }
}

/// Meetings a transcription pass completed for this session: the server-side evidence
/// [`reevaluate_after_backlog`] requires before it treats a sparse transcript as final.
static TRANSCRIPT_PASSES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

fn record_transcript_pass(meeting_id: &str) {
    if let Ok(mut set) = TRANSCRIPT_PASSES.lock() {
        set.insert(meeting_id.to_string());
    }
}

fn take_transcript_pass(meeting_id: &str) -> bool {
    TRANSCRIPT_PASSES
        .lock()
        .map(|mut set| set.remove(meeting_id))
        .unwrap_or(false)
}

/// A transcription pass replaced the meeting's transcript rows (`retranscription.rs`, with
/// `segments` rows). Clears the deferred marker (every surface's pass is final, 1.10
/// feedback) and the stale speaker labels. A pending meeting may now be processed — but a
/// sparse result (below `MIN_TRANSCRIPT_SEGMENTS`, e.g. an engine misfire) only counts as
/// transcribed when the pass was the deliberate deferred-processing run, i.e. the meeting
/// carried a `defer`/`live` marker. Otherwise it stays pending and keeps its audio.
pub async fn on_transcript_replaced<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    segments: usize,
) {
    let Some(pool) = pool(app) else { return };
    let row = state::load_row(&pool, meeting_id).await.ok().flatten();
    let marked = row
        .as_ref()
        .is_some_and(|r| matches!(r.processing_mode.as_deref(), Some("defer") | Some("live")));
    crate::audio::retranscription::clear_deferred_marker_after_transcription(&pool, meeting_id)
        .await;
    if let Err(e) = state::clear_speakers_identified(&pool, meeting_id).await {
        log::warn!("Audio lifecycle: {e:#}");
    }
    record_transcript_pass(meeting_id);
    if !row.is_some_and(|r| r.state() == AudioState::Pending) {
        return;
    }
    if marked || segments as i64 >= super::MIN_TRANSCRIPT_SEGMENTS {
        reevaluate_and_apply(app, meeting_id, true).await;
    } else {
        log::info!(
            "Audio lifecycle: meeting {meeting_id} transcribed to {segments} segment(s) outside \
             deferred processing; keeping it pending"
        );
    }
}

/// The backlog cleared a meeting's `defer` marker (`api_set_meeting_processing_mode(None)`).
/// The caller is not trusted to mean "transcription is done": a sparse transcript counts as
/// final only when a transcription pass completed for this meeting in this session
/// ([`on_transcript_replaced`] ran). Otherwise the meeting is reevaluated normally, and a
/// sparse one stays pending (the backlog offers it again).
pub async fn reevaluate_after_backlog<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    let transcribed = take_transcript_pass(meeting_id);
    reevaluate_and_apply(app, meeting_id, transcribed).await;
}

/// A resumed recording starts a new segment (called under the meeting's Recording lease).
pub async fn reset_for_resume<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    let Some(pool) = pool(app) else { return };
    match state::reset_for_resume(&pool, meeting_id).await {
        Ok(true) => emit_state(app, meeting_id, AudioState::Pending),
        Ok(false) => {}
        Err(e) => log::warn!("Audio lifecycle: {e:#}"),
    }
}

/// Run a sweep with the saved policy and emit every purge. `compress` = false for the
/// user's "apply now" (they are waiting on the toast).
pub async fn sweep_now<R: Runtime>(
    app: &AppHandle<R>,
    opts: &SweepOptions,
) -> Result<RetentionReport, String> {
    let pool = pool(app).ok_or("The database isn't ready yet.")?;
    let policy = saved_policy(app)
        .await
        .ok_or("Nixon couldn't read your audio storage setting, so nothing was deleted.")?;
    let report = sweep::run_sweep(&pool, policy, chrono::Utc::now(), opts)
        .await
        .map_err(|e| format!("Couldn't clean up audio: {e:#}"))?;
    for id in &report.purged_ids {
        emit_state(app, id, AudioState::Purged);
    }
    Ok(report)
}

/// Finish every meeting a quit interrupted mid-processing, one at a time.
async fn startup_reconcile<R: Runtime>(app: &AppHandle<R>) {
    let Some(pool) = pool(app) else { return };
    let ids = match state::list_unfinished(&pool).await {
        Ok(ids) => ids,
        Err(e) => return log::warn!("Audio lifecycle reconcile: {e:#}"),
    };
    for id in ids {
        if crate::audio::folder_lease::current_holder(&id).is_some() {
            continue;
        }
        if finish_processing(app, &id, false).await == FinishOutcome::Diarizing {
            wait_for_diarization(&id).await;
        }
        if matches!(state::load_row(&pool, &id).await, Ok(Some(r)) if r.state() == AudioState::Pending)
        {
            log::warn!(
                "Audio lifecycle reconcile: meeting {id} is still pending; its audio is kept"
            );
        }
    }
}

async fn wait_for_diarization(meeting_id: &str) {
    let deadline = tokio::time::Instant::now() + RECONCILE_DIARIZATION_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let running =
            crate::diarization::pipeline::run_status(meeting_id).is_some_and(|s| s.running);
        if !running {
            return;
        }
    }
}

/// Start the lifecycle: the startup reconcile, and the sweep at +60 s then hourly.
/// Replaces the 0029 retention sweeper. Spawn after database init.
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    let reconcile_app = app.clone();
    tauri::async_runtime::spawn(async move { startup_reconcile(&reconcile_app).await });
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_SWEEP_DELAY).await;
        loop {
            if pool(&app).is_some() {
                if let Err(e) = sweep_now(&app, &scheduled_options().await).await {
                    log::warn!("Audio sweep failed: {e}");
                }
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}
