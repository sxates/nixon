//! Tauri commands for the diarization ONNX model lifecycle (specs/0010, ADR-0005).
//!
//! Split out of [`crate::diarization::commands`] (specs/0059) purely to keep that
//! file under the repo's file-size ratchet — no behaviour change:
//! - [`api_download_diarization_models`] — explicit first-run model fetch.
//! - [`api_diarization_models_present`] — whether both models are cached.

use tauri::{AppHandle, Emitter, Runtime};

use crate::diarization::models;
use crate::diarization::pipeline::EVENT_PROGRESS;

/// New in specs/0061 W2: byte-level download progress, so a persistent frontend
/// toast can show a moving MB counter instead of going silent for the ~108 MB
/// embedding-model fetch. `stage` is the fully formatted [`models::progress_label`]
/// text (already includes the MB fraction, or is stage-only when `total_bytes`
/// is 0/unknown) so the frontend needs no byte-math of its own.
const EVENT_DOWNLOAD_PROGRESS: &str = "diarization-download-progress";

/// Download the two diarization ONNX models on demand, emitting
/// `diarization-progress` per stage (compatibility) and the byte-level
/// `diarization-download-progress` (specs/0061 W2). No-op if already cached.
#[tauri::command]
pub async fn api_download_diarization_models<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    #[cfg(debug_assertions)]
    if crate::dev_fixtures::fake_downloads::active() {
        crate::dev_fixtures::fake_downloads::run(
            &app,
            crate::dev_fixtures::fake_downloads::Model::Diarization,
            "diarization",
        )
        .await;
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let progress = move |stage: models::DownloadStage, downloaded: u64, total: u64| {
            let label = models::progress_label(stage, downloaded, total);
            let _ = app.emit(EVENT_PROGRESS, serde_json::json!({ "stage": label }));
            let _ = app.emit(
                EVENT_DOWNLOAD_PROGRESS,
                serde_json::json!({
                    "stage": label,
                    "downloaded_bytes": downloaded,
                    "total_bytes": total,
                }),
            );
        };
        models::ensure_models(Some(&progress)).map(|_| ())
    })
    .await
    .map_err(|e| format!("model download task panicked: {e}"))?
    .map_err(|e| format!("Failed to download diarization models: {e:#}"))
}

/// Whether both diarization models are present (and a plausible size) on disk.
#[tauri::command]
pub async fn api_diarization_models_present() -> Result<bool, String> {
    #[cfg(debug_assertions)]
    if crate::dev_fixtures::fake_downloads::is_faked_present(
        crate::dev_fixtures::fake_downloads::Model::Diarization,
    ) {
        return Ok(true);
    }
    Ok(models::models_present())
}
