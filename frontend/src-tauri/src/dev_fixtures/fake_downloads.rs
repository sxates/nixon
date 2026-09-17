//! Simulated model downloads for the onboarding harness (specs/0059). Emits the same events
//! the real downloads do so the frontend needs no changes; nothing is written to disk.
use super::guard;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Parakeet,
    BuiltinAi,
    Diarization,
}

static PRESENT: [AtomicBool; 3] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];
const TOTAL_SECONDS: u64 = 10;
const STEPS: usize = 40;

pub fn active() -> bool {
    guard::env_flag(guard::ENV_FAKE_DOWNLOADS) && guard::is_debug_identifier()
}

pub fn is_faked_present(m: Model) -> bool {
    PRESENT[m as usize].load(Ordering::Relaxed)
}
pub fn mark_present(m: Model) {
    PRESENT[m as usize].store(true, Ordering::Relaxed);
}

/// Ease-out curve, 0..=100, `n` points, last is exactly 100.
pub fn progress_curve(n: usize) -> Vec<u8> {
    (1..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            ((1.0 - (1.0 - t).powi(2)) * 100.0).round() as u8
        })
        .collect()
}

pub async fn run<R: Runtime>(app: &AppHandle<R>, model: Model, name: &str) {
    let total_mb: f64 = match model {
        Model::Parakeet => 670.0,
        Model::BuiltinAi => 1221.0,
        Model::Diarization => 35.0,
    };
    let tick = std::time::Duration::from_millis(TOTAL_SECONDS * 1000 / STEPS as u64);
    for pct in progress_curve(STEPS) {
        let done_mb = total_mb * pct as f64 / 100.0;
        match model {
            Model::Parakeet => {
                let _ = app.emit(
                    "parakeet-model-download-progress",
                    serde_json::json!({
                        "modelName": name, "progress": pct, "downloaded_bytes": (done_mb * 1_048_576.0) as u64, "total_bytes": (total_mb * 1_048_576.0) as u64,
                        "downloaded_mb": done_mb, "total_mb": total_mb, "speed_mbps": 64.0, "status": if pct == 100 { "completed" } else { "downloading" } }),
                );
            }
            Model::BuiltinAi => {
                let _ = app.emit(
                    "builtin-ai-download-progress",
                    serde_json::json!({
                        "model": name, "progress": pct, "downloaded_mb": done_mb, "total_mb": total_mb, "speed_mbps": 64.0, "status": "downloading" }),
                );
            }
            Model::Diarization => {
                let stage = if pct < 40 {
                    "downloading segmentation model"
                } else if pct < 90 {
                    "downloading embedding model"
                } else {
                    "extracting model"
                };
                let _ = app.emit(
                    "diarization-progress",
                    serde_json::json!({ "stage": stage }),
                );
            }
        }
        tokio::time::sleep(tick).await;
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await; // "verifying" tail
    mark_present(model);
    match model {
        Model::Parakeet => {
            let _ = app.emit(
                "parakeet-model-download-complete",
                serde_json::json!({ "modelName": name }),
            );
        }
        Model::BuiltinAi => {
            let _ = app.emit(
                "builtin-ai-download-progress",
                serde_json::json!({ "model": name, "progress": 100, "downloaded_mb": total_mb, "total_mb": total_mb, "speed_mbps": 0.0, "status": "completed" }),
            );
        }
        Model::Diarization => {}
    }
    log::info!("[dev] simulated download complete: {name} (simulated)");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_is_monotonic_and_ends_at_100() {
        let pts = progress_curve(10);
        assert_eq!(pts.len(), 10);
        assert!(pts.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*pts.last().unwrap(), 100);
    }
    // `PRESENT` is a process-global static, so this test's assertions on
    // `Model::Parakeet` are only valid if no other test in this process also
    // touches `Model::Parakeet`'s flag. Any future test needing a fresh
    // "unset" flag must use `Model::BuiltinAi` or `Model::Diarization`.
    #[test]
    fn present_flags_default_false_and_set() {
        assert!(!is_faked_present(Model::Parakeet));
        mark_present(Model::Parakeet);
        assert!(is_faked_present(Model::Parakeet));
    }
}
