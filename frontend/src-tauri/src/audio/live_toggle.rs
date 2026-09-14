//! Session-scoped live-STT toggle state (low-power-mode spec §§3–4).
//!
//! Bridges three parties without growing the (ratchet-frozen) recording
//! commands file: the start path decides the initial mode and stashes the
//! transcription receiver when deferred; the running pipeline polls the
//! session flag each window (attach/detach VAD mid-recording); the
//! `api_apply_live_transcription_now` command flips the flag and spawns the
//! worker on first enable.

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::common::TranscriptionChunk;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;

/// Receiver kept alive while a deferred session runs, so a mid-meeting
/// "go live" can hand it to the transcription worker. Dropped at stop.
static PENDING_RECEIVER: Lazy<Mutex<Option<mpsc::Receiver<TranscriptionChunk>>>> =
    Lazy::new(|| Mutex::new(None));

/// The active session's live-STT flag (shared with the pipeline). `None`
/// outside a recording session.
static SESSION_FLAG: Lazy<Mutex<Option<Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(None));

/// The live-transcription worker's task handle. Held here (rather than in the
/// ratchet-frozen `recording_commands.rs`) so both the start/stop paths there AND
/// the mid-meeting go-live command below can store/take it. Spawned on first enable
/// (start-time for a live session, or the toggle command for a deferred one), taken
/// and awaited at stop.
static TRANSCRIPTION_TASK: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub fn store_transcription_task(handle: JoinHandle<()>) {
    *TRANSCRIPTION_TASK.lock().unwrap() = Some(handle);
}

pub fn take_transcription_task() -> Option<JoinHandle<()>> {
    TRANSCRIPTION_TASK.lock().unwrap().take()
}

pub fn stash_receiver(receiver: mpsc::Receiver<TranscriptionChunk>) {
    *PENDING_RECEIVER.lock().unwrap() = Some(receiver);
}

pub fn take_receiver() -> Option<mpsc::Receiver<TranscriptionChunk>> {
    PENDING_RECEIVER.lock().unwrap().take()
}

pub fn set_session_flag(flag: Arc<AtomicBool>) {
    *SESSION_FLAG.lock().unwrap() = Some(flag);
}

/// Build the session's shared live-STT flag from the start-time mode and register it
/// as the active session flag (so the mid-meeting toggle can flip it). Returns the
/// `Arc` to hand to the pipeline. Called from both recording start paths.
pub fn register_session_flag(initial: bool) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(initial));
    set_session_flag(flag.clone());
    flag
}

pub fn session_flag() -> Option<Arc<AtomicBool>> {
    SESSION_FLAG.lock().unwrap().clone()
}

/// True when the ACTIVE session is currently running live STT.
pub fn session_live_now() -> Option<bool> {
    session_flag().map(|f| f.load(Ordering::SeqCst))
}

/// Stop-time cleanup: drop the flag and any unconsumed receiver.
pub fn clear_session() {
    *SESSION_FLAG.lock().unwrap() = None;
    *PENDING_RECEIVER.lock().unwrap() = None;
}

/// Decide the session's initial live/defer mode (spec §§2–4) and tell the
/// frontend. Called from both recording start paths.
pub async fn decide_session_mode<R: Runtime>(
    app: &AppHandle<R>,
    prefs_live_transcription: bool,
    prefs_low_power_on_battery: bool,
    meeting_id: Option<&str>,
) -> bool {
    let on_battery = crate::power::is_on_battery();
    let override_mode = match meeting_id {
        Some(mid) => {
            let state = app.state::<AppState>();
            let pool = state.db_manager.pool();
            MeetingsRepository::get_processing_mode(pool, mid)
                .await
                .unwrap_or_default()
        }
        None => None,
    };
    let live = super::processing_mode::effective_live_stt(
        prefs_live_transcription,
        prefs_low_power_on_battery,
        on_battery,
        override_mode.as_deref(),
    );
    log::info!(
        "processing mode: live={live} (pref_live={prefs_live_transcription}, low_power={prefs_low_power_on_battery}, on_battery={on_battery}, override={override_mode:?})"
    );
    let _ = app.emit(
        "processing-mode-changed",
        serde_json::json!({
            "meetingId": meeting_id,
            "liveTranscription": live,
            "onBattery": on_battery,
        }),
    );
    live
}

/// Flip live transcription for the ACTIVE recording session (spec §3).
/// Enabling validates the model, spawns the transcription worker on first
/// use (handing it the stashed receiver), then raises the flag; the pipeline
/// attaches its VAD on the next mix window. Disabling just lowers the flag
/// (the worker idles; the engine stays resident — CPU, not RAM, is the
/// battery cost). Persisting the meeting's `processing_mode` override is the
/// frontend's separate `api_set_meeting_processing_mode` call.
#[tauri::command]
pub async fn api_apply_live_transcription_now<R: Runtime>(
    app: AppHandle<R>,
    enable: bool,
) -> Result<(), String> {
    let Some(flag) = session_flag() else {
        return Err("No recording in progress".to_string());
    };
    if enable {
        super::transcription::validate_transcription_model_ready(&app).await?;
        // Spawn the worker only if a deferred session stashed a receiver. Idempotent
        // across enable→disable→enable: the second enable finds no stashed receiver
        // (already consumed) and just re-raises the flag; the worker is still resident.
        if let Some(receiver) = take_receiver() {
            let handle = super::transcription::start_transcription_task(app.clone(), receiver);
            store_transcription_task(handle);
        }
        flag.store(true, Ordering::SeqCst);
    } else {
        flag.store(false, Ordering::SeqCst);
    }
    let _ = app.emit(
        "processing-mode-changed",
        serde_json::json!({
            "liveTranscription": enable,
            "onBattery": crate::power::is_on_battery(),
        }),
    );
    Ok(())
}

/// Frontend query: a snapshot of the ACTIVE session's processing state, for
/// late-mount / remount hydration (low-power-mode spec §5, Finding 2). The
/// `processing-mode-changed` event stream is fire-once, so a `useProcessingMode`
/// consumer that mounts after the start-time emit (navigating back to /record,
/// or an auto-started recording that began while the user was elsewhere) would
/// otherwise never learn the session's mode. Returns:
///  - `active`: a recording session is in progress (`session_flag().is_some()`);
///  - `liveTranscription`: whether live STT is on right now, or `null` when inactive;
///  - `onBattery`: the current power source.
#[tauri::command]
pub async fn api_get_session_processing_state() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "active": session_flag().is_some(),
        "liveTranscription": session_live_now(),
        "onBattery": crate::power::is_on_battery(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One combined test: these helpers share process-wide statics, and cargo
    /// runs same-binary tests on parallel threads — two tests each calling
    /// `clear_session()` can wipe each other's state mid-assertion. Keeping
    /// every static round-trip in a single #[test] serializes them by
    /// construction.
    #[test]
    fn session_statics_round_trip() {
        clear_session();
        assert!(take_receiver().is_none(), "no receiver stashed yet");

        let (_tx, rx) = mpsc::channel::<TranscriptionChunk>(1);
        stash_receiver(rx);
        assert!(
            take_receiver().is_some(),
            "receiver should be returned once"
        );
        assert!(
            take_receiver().is_none(),
            "receiver should be consumed by the first take"
        );

        clear_session();
        assert!(session_flag().is_none());
        assert!(session_live_now().is_none());

        let flag = Arc::new(AtomicBool::new(true));
        set_session_flag(flag.clone());
        assert_eq!(session_live_now(), Some(true));

        flag.store(false, Ordering::SeqCst);
        assert_eq!(session_live_now(), Some(false));

        clear_session();
        assert!(session_flag().is_none());
    }
}
