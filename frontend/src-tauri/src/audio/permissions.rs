// macOS audio permissions handling
use anyhow::Result;
use log::{error, info, warn};
use serde::Serialize;

#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
use std::pin::Pin;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

use super::probe_tone;

/// Check if the app has Audio Capture permission (required for Core Audio taps on macOS 14.4+)
///
/// Note: Core Audio taps require NSAudioCaptureUsageDescription in Info.plist.
/// When the app first attempts to create a Core Audio tap, macOS will automatically
/// show a permission dialog to the user. If permission is denied, the tap will return
/// silence (all zeros).
///
/// This function returns true because the actual permission prompt happens automatically
/// when AudioHardwareCreateProcessTap is called by the cidre library.
#[cfg(target_os = "macos")]
pub fn check_audio_capture_permission() -> bool {
    info!("ℹ️  Core Audio tap requires Audio Capture permission (macOS 14.4+)");
    info!("📍 Permission dialog will appear automatically when recording starts");
    info!("   If already granted: System Settings → Privacy & Security → Audio Capture");

    // Always return true - the actual permission dialog is triggered by Core Audio API
    true
}

#[cfg(not(target_os = "macos"))]
pub fn check_audio_capture_permission() -> bool {
    true // Not required on other platforms
}

/// Request Audio Capture permission from the user
/// This will open System Settings to the Privacy & Security page
#[cfg(target_os = "macos")]
pub fn request_audio_capture_permission() -> Result<()> {
    info!("🔐 Opening System Settings for Audio Capture permission...");

    // Open System Settings to Privacy & Security page
    // Note: There's no direct URL for Audio Capture, so we open the main Privacy page
    let result = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security")
        .spawn();

    match result {
        Ok(_) => {
            info!("✅ Opened System Settings - navigate to Privacy & Security → Audio Capture");
            info!("👉 Please enable Audio Capture permission and restart the app");
            Ok(())
        }
        Err(e) => {
            error!("❌ Failed to open System Settings: {}", e);
            Err(anyhow::anyhow!("Failed to open System Settings: {}", e))
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_audio_capture_permission() -> Result<()> {
    Ok(()) // Not required on other platforms
}

/// Tauri command to check Audio Capture permission
#[tauri::command]
pub async fn check_audio_capture_permission_command() -> bool {
    check_audio_capture_permission()
}

/// Tauri command to request Audio Capture permission
#[tauri::command]
pub async fn request_audio_capture_permission_command() -> Result<(), String> {
    request_audio_capture_permission().map_err(|e| e.to_string())
}

/// Result of probing the Audio Capture tap with real audio (specs/0061 W3).
///
/// Replaces the old boolean "did the tap construct?" signal, which lied to
/// onboarding: tap construction succeeds even when permission is denied (the
/// tap just delivers silence). The onboarding UI and the "Enable recording"
/// modal both key their copy off `state`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PermissionProbe {
    /// The tap delivered real audio above the noise floor — Audio Capture is
    /// genuinely granted.
    Granted,
    /// The tap opened but only ever delivered silence — permission is most
    /// likely not granted yet (macOS may still prompt on the first real
    /// recording).
    Silent,
    /// The tap itself could not be created or started.
    Failed { message: String },
}

/// Classify a probe result from its measured RMS and any construction error.
/// Pure function — no I/O — so it is fully unit-testable without hardware.
pub fn classify(rms: f32, err: Option<String>) -> PermissionProbe {
    if let Some(message) = err {
        return PermissionProbe::Failed { message };
    }
    if rms > 1e-4 {
        PermissionProbe::Granted
    } else {
        PermissionProbe::Silent
    }
}

/// How long to let the probe tone play (slightly longer than the capture
/// window below, to cover thread-start/device-start latency at both ends).
#[cfg(target_os = "macos")]
const PROBE_TONE_MS: u32 = 600;

/// How long to collect samples from the tap before computing RMS.
#[cfg(target_os = "macos")]
const PROBE_CAPTURE_MS: u64 = 400;

/// Root-mean-square of a sample buffer; `0.0` for an empty buffer (never
/// `NaN`).
#[cfg(target_os = "macos")]
fn rms_of(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Drain up to `duration` worth of samples from a `CoreAudioStream` by
/// manually polling it — no tokio/async executor required, so this can run
/// entirely inside a plain blocking thread. `CoreAudioStream` is `Unpin` (its
/// fields are all plain owned types), so this needs no `unsafe`.
///
/// Bounded strictly by wall-clock `duration`: a `Poll::Pending` just means a
/// short sleep before the next poll, it never blocks past the deadline. This
/// is what keeps the probe from hanging onboarding even if the tap never
/// delivers a sample.
#[cfg(target_os = "macos")]
fn collect_samples(
    stream: &mut crate::audio::capture::CoreAudioStream,
    duration: Duration,
) -> Vec<f32> {
    use futures_util::task::noop_waker_ref;
    use futures_util::Stream;
    use std::task::{Context, Poll};

    let mut samples = Vec::new();
    let deadline = Instant::now() + duration;
    let waker = noop_waker_ref();
    let mut cx = Context::from_waker(waker);
    let mut pinned = Pin::new(stream);

    while Instant::now() < deadline {
        match pinned.as_mut().poll_next(&mut cx) {
            Poll::Ready(Some(sample)) => samples.push(sample),
            Poll::Ready(None) => break,
            Poll::Pending => std::thread::sleep(Duration::from_millis(1)),
        }
    }

    samples
}

/// Synchronous body of the Audio Capture probe: create the tap, play a
/// brief quiet tone on a separate thread, collect ~400ms of samples from the
/// tap, and classify the result. Entirely synchronous by design (see
/// `trigger_system_audio_permission_command`) so it can run on a
/// `spawn_blocking` thread without requiring `CoreAudioCapture`/
/// `CoreAudioStream` to be `Send` across an `.await` point.
#[cfg(target_os = "macos")]
fn probe_audio_capture_sync() -> PermissionProbe {
    let capture = match crate::audio::capture::CoreAudioCapture::new() {
        Ok(c) => c,
        Err(e) => {
            warn!("Audio Capture probe: failed to create tap: {e}");
            return classify(0.0, Some(e.to_string()));
        }
    };

    let mut stream = match capture.stream() {
        Ok(s) => s,
        Err(e) => {
            warn!("Audio Capture probe: failed to start stream: {e}");
            return classify(0.0, Some(e.to_string()));
        }
    };

    // Play a brief, quiet tone on its own thread so the tap has real audio to
    // pick up. `play_440_for_ms` swallows its own device errors; if it can't
    // play anything, the tap will simply read silence and we classify that
    // honestly as `Silent`, not `Failed` — a muted/absent output device is
    // not the same failure as a tap that couldn't be created.
    std::thread::spawn(|| probe_tone::play_440_for_ms(PROBE_TONE_MS));

    let samples = collect_samples(&mut stream, Duration::from_millis(PROBE_CAPTURE_MS));
    let rms = rms_of(&samples);
    info!(
        "Audio Capture probe: collected {} samples over {}ms, rms={:.6}",
        samples.len(),
        PROBE_CAPTURE_MS,
        rms
    );
    classify(rms, None)
}

/// Tauri command: probe the Audio Capture tap with a real (quiet, brief)
/// tone and report what actually came back, rather than assuming grant from
/// tap construction alone (specs/0061 W3 — this is the fix for the lie that
/// let a user finish onboarding believing system audio worked when it did
/// not).
///
/// The whole probe — tap creation, tone playback, sample collection,
/// classification — runs under a 2s timeout so a stuck tap can never hang
/// onboarding; a timeout is reported as `Failed`, same as a real probe
/// error, since from the frontend's point of view both mean "couldn't
/// verify audio capture right now."
#[tauri::command]
pub async fn trigger_system_audio_permission_command() -> Result<PermissionProbe, String> {
    #[cfg(target_os = "macos")]
    {
        match tokio::time::timeout(
            Duration::from_secs(2),
            tokio::task::spawn_blocking(probe_audio_capture_sync),
        )
        .await
        {
            Ok(Ok(probe)) => Ok(probe),
            Ok(Err(join_err)) => Ok(PermissionProbe::Failed {
                message: format!("Audio Capture probe task failed: {join_err}"),
            }),
            Err(_) => {
                warn!("Audio Capture probe timed out after 2s");
                Ok(PermissionProbe::Failed {
                    message: "Audio Capture probe timed out".to_string(),
                })
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        info!("Audio Capture permission not required on this platform");
        Ok(PermissionProbe::Granted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_permission() {
        let has_permission = check_audio_capture_permission();
        println!("Has Audio Capture permission: {}", has_permission);
    }

    #[test]
    fn classify_granted_when_rms_above_threshold() {
        assert!(matches!(classify(0.01, None), PermissionProbe::Granted));
    }

    #[test]
    fn classify_silent_when_rms_at_or_below_threshold() {
        assert!(matches!(classify(0.0, None), PermissionProbe::Silent));
    }

    #[test]
    fn classify_failed_when_err_present_regardless_of_rms() {
        match classify(0.0, Some("x".to_string())) {
            PermissionProbe::Failed { message } => assert_eq!(message, "x"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
