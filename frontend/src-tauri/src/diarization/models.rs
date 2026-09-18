//! On-demand download + cache of the diarization ONNX models. (specs/0010)
//!
//! Two models, cached under `app_data_dir()/models/diarization/` (resolved via
//! the Tauri-path-derived [`crate::app_paths::app_data_dir`], never hardcoded —
//! mirrors the Parakeet/Whisper download pattern):
//!
//! - **segmentation** (pyannote `segmentation-3.0`, MIT, ~6.6 MB): shipped as a
//!   `.tar.bz2` whose `model.onnx` we extract to `segmentation.onnx`.
//! - **embedding** (NVIDIA NeMo TitaNet-L, CC-BY-4.0, ~101 MB): a bare `.onnx`.
//!   Replaced 3D-Speaker CAM++ in specs/0043 W2.2 — measured macro DER on the
//!   ground-truth harness dropped 32.8% → 8.1% (ADR-0011). A leftover CAM++
//!   model file on disk is simply ignored (harmless ~28 MB).
//!
//! Total ~108 MB; downloaded on demand, not bundled (ADR-0005 decision 4).
//!
//! P1-A provides [`models_present`] and [`ensure_models`]; the latter takes an
//! optional progress callback so the IPC/event slice (P1-B) can surface a
//! `diarization-progress` event without re-plumbing this code.

use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Filename of the extracted segmentation model on disk.
pub const SEGMENTATION_MODEL_FILE: &str = "segmentation.onnx";
/// Filename of the speaker-embedding model on disk (specs/0043 W2.2: TitaNet-L).
pub const EMBEDDING_MODEL_FILE: &str = "nemo_en_titanet_large.onnx";

/// Download URL for the pyannote segmentation-3.0 tarball (verified, ADR-0005).
const SEGMENTATION_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2";
/// Download URL for the NeMo TitaNet-L embedding model (verified, ADR-0011).
const EMBEDDING_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/nemo_en_titanet_large.onnx";

/// Loose lower bounds for an integrity sanity-check (not a checksum — just catches
/// truncated/empty/HTML-error downloads). Real sizes: seg ≈ 5.7 MB, emb ≈ 101 MB.
const SEGMENTATION_MIN_BYTES: u64 = 4 * 1024 * 1024;
const EMBEDDING_MIN_BYTES: u64 = 90 * 1024 * 1024;

/// Resolved paths to the two models (whether or not they exist yet).
#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub dir: PathBuf,
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
}

/// Stage of [`ensure_models`], for progress reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStage {
    Segmentation,
    Embedding,
    Extracting,
}

/// The diarization model cache directory: `app_data_dir()/models/diarization/`.
pub fn models_dir() -> PathBuf {
    crate::app_paths::app_data_dir()
        .join("models")
        .join("diarization")
}

/// Resolve the model paths under [`models_dir`].
pub fn model_paths() -> ModelPaths {
    let dir = models_dir();
    ModelPaths {
        segmentation: dir.join(SEGMENTATION_MODEL_FILE),
        embedding: dir.join(EMBEDDING_MODEL_FILE),
        dir,
    }
}

/// Whether both models exist on disk with a plausible (non-truncated) size.
pub fn models_present() -> bool {
    let p = model_paths();
    file_at_least(&p.segmentation, SEGMENTATION_MIN_BYTES)
        && file_at_least(&p.embedding, EMBEDDING_MIN_BYTES)
}

fn file_at_least(path: &Path, min: u64) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() >= min)
        .unwrap_or(false)
}

/// Human-readable label for a download stage + byte-progress pair (specs/0061
/// W2). Pure and side-effect-free so it's unit-testable without a network call;
/// the Tauri command layer sends its output straight through as UI copy.
/// `total == 0` means "unknown" (headers didn't report `Content-Length`, or the
/// stage does no streaming at all, e.g. extraction) — degrade to stage-only text
/// rather than showing a "0 MB" or "NaN" fraction.
pub fn progress_label(stage: DownloadStage, downloaded: u64, total: u64) -> String {
    let stage_text = match stage {
        DownloadStage::Segmentation => "downloading segmentation model",
        DownloadStage::Embedding => "downloading embedding model",
        DownloadStage::Extracting => "extracting model",
    };
    if total == 0 {
        return stage_text.to_string();
    }
    let downloaded_mb = downloaded as f64 / (1024.0 * 1024.0);
    let total_mb = total as f64 / (1024.0 * 1024.0);
    format!("{stage_text} · {downloaded_mb:.1} MB / {total_mb:.1} MB")
}

/// Ensure both models are present, downloading any that are missing.
///
/// Returns the resolved [`ModelPaths`]. Idempotent: present-and-valid models are
/// left untouched. `progress` (if any) is called with `(stage, downloaded_bytes,
/// total_bytes)` throughout each download (`total_bytes` is 0 until the response
/// headers are read, and stays 0 for the whole call if the server never sent
/// `Content-Length`) and once with `(Extracting, 0, 0)`, so later slices can
/// emit UI events; pass `None` for a silent run.
///
/// This is a **blocking** call (uses `reqwest::blocking`, matching how the
/// build-time ffmpeg fetch and the synchronous parts of model setup work). Run it
/// off the UI thread (e.g. `tauri::async_runtime::spawn_blocking`) in the
/// orchestration slice.
pub fn ensure_models(progress: Option<&dyn Fn(DownloadStage, u64, u64)>) -> Result<ModelPaths> {
    let paths = model_paths();
    std::fs::create_dir_all(&paths.dir)
        .with_context(|| format!("create diarization models dir {}", paths.dir.display()))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .context("build HTTP client for diarization model download")?;

    if !file_at_least(&paths.segmentation, SEGMENTATION_MIN_BYTES) {
        log::info!("Downloading diarization segmentation model…");
        let bytes = download(&client, SEGMENTATION_URL, |downloaded, total| {
            if let Some(cb) = progress {
                cb(DownloadStage::Segmentation, downloaded, total);
            }
        })?;
        if let Some(cb) = progress {
            cb(DownloadStage::Extracting, 0, 0);
        }
        extract_segmentation_onnx(&bytes, &paths.segmentation)?;
        if !file_at_least(&paths.segmentation, SEGMENTATION_MIN_BYTES) {
            return Err(anyhow!(
                "segmentation model at {} is missing or too small after extraction",
                paths.segmentation.display()
            ));
        }
        log::info!("Segmentation model ready: {}", paths.segmentation.display());
    }

    if !file_at_least(&paths.embedding, EMBEDDING_MIN_BYTES) {
        log::info!("Downloading diarization embedding model…");
        let bytes = download(&client, EMBEDDING_URL, |downloaded, total| {
            if let Some(cb) = progress {
                cb(DownloadStage::Embedding, downloaded, total);
            }
        })?;
        write_atomic(&paths.embedding, &bytes)?;
        if !file_at_least(&paths.embedding, EMBEDDING_MIN_BYTES) {
            return Err(anyhow!(
                "embedding model at {} is missing or too small after download",
                paths.embedding.display()
            ));
        }
        log::info!("Embedding model ready: {}", paths.embedding.display());
    }

    Ok(paths)
}

/// A ~1 MiB throttle for progress reporting (specs/0061 W2 fix round 2, R21):
/// the raw read loop below yields 64 KiB chunks (1600+ per ~101 MB model), and
/// firing `on_chunk` on every one of them means 1600+ IPC emits and React state
/// updates, plus (via the pipeline.rs caller) 1600+ registry locks, for a single
/// download. Report at this granularity instead — still far finer than the
/// single stage string that shipped before, and cheap enough not to matter.
const PROGRESS_REPORT_STEP_BYTES: u64 = 1024 * 1024;

/// Pure check for a truncated download (specs/0061 W2 fix round 2, R20):
/// `total == 0` means the server never sent (or we never read) a
/// `Content-Length`, so there's nothing to compare against — not an error.
/// Only `downloaded < total` is flagged: a gzip-encoded body legitimately
/// decompresses to *more* bytes than the (compressed) `Content-Length`, so a
/// `!=` check would spuriously fail a perfectly good download.
fn check_download_complete(url: &str, downloaded: u64, total: u64) -> Result<()> {
    if total > 0 && downloaded < total {
        return Err(anyhow!(
            "short read for {url}: expected {total} bytes, got {downloaded} (connection likely dropped)"
        ));
    }
    Ok(())
}

/// Stream the response body, calling `on_chunk(downloaded_so_far, total)` at
/// roughly [`PROGRESS_REPORT_STEP_BYTES`] granularity (plus always once more at
/// completion, so the UI never stalls just short of done) so a caller can
/// surface byte-level progress (specs/0061 W2) instead of blocking silently
/// until the whole ~101 MB body has arrived. Errors if the connection closes
/// (a clean EOF) before `Content-Length` bytes were read — see
/// [`check_download_complete`].
fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    mut on_chunk: impl FnMut(u64, u64),
) -> Result<Vec<u8>> {
    let mut resp = client
        .get(url)
        .send()
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("download failed for {url}"))?;
    let total = resp.content_length().unwrap_or(0);
    let mut buf = Vec::with_capacity(total as usize);
    let mut chunk = [0u8; 64 * 1024];
    let mut last_reported: u64 = 0;
    loop {
        let n = resp
            .read(&mut chunk)
            .with_context(|| format!("read body of {url}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let downloaded = buf.len() as u64;
        if downloaded - last_reported >= PROGRESS_REPORT_STEP_BYTES {
            on_chunk(downloaded, total);
            last_reported = downloaded;
        }
    }
    // Always report the final byte count, even if it fell short of a full
    // PROGRESS_REPORT_STEP_BYTES step since the last report, so the UI reflects
    // reality (including a short read) rather than stalling just short of done.
    on_chunk(buf.len() as u64, total);
    check_download_complete(url, buf.len() as u64, total)?;
    Ok(buf)
}

/// Atomically write `bytes` to `path` (write to a temp sibling, then rename) so a
/// crash mid-write never leaves a half-written model that passes a size check.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("partial");
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("write {}", tmp.display()))?;
        f.flush().ok();
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Extract the `model.onnx` from the segmentation `.tar.bz2` archive into `dest`.
///
/// The sherpa release lays the tarball out as
/// `sherpa-onnx-pyannote-segmentation-3-0/model.onnx`; we take the first entry
/// whose filename is `model.onnx`.
fn extract_segmentation_onnx(tar_bz2: &[u8], dest: &Path) -> Result<()> {
    let decoder = bzip2::read::BzDecoder::new(tar_bz2);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("read segmentation tar entries")? {
        let mut entry = entry.context("read segmentation tar entry")?;
        let path = entry.path().context("entry path")?.into_owned();
        if path.file_name().and_then(|n| n.to_str()) == Some("model.onnx") {
            let mut buf = Vec::new();
            std::io::copy(&mut entry, &mut buf).context("read model.onnx from tar")?;
            write_atomic(dest, &buf)?;
            return Ok(());
        }
    }
    Err(anyhow!("model.onnx not found inside segmentation tarball"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_under_diarization_subdir() {
        let p = model_paths();
        assert!(p.dir.ends_with("models/diarization"));
        assert_eq!(p.segmentation.file_name().unwrap(), SEGMENTATION_MODEL_FILE);
        assert_eq!(p.embedding.file_name().unwrap(), EMBEDDING_MODEL_FILE);
    }

    #[test]
    fn missing_files_report_not_present() {
        // In a clean test env the models are not downloaded; just assert the
        // function doesn't panic and returns a bool consistent with disk state.
        let present = models_present();
        let p = model_paths();
        let expected = file_at_least(&p.segmentation, SEGMENTATION_MIN_BYTES)
            && file_at_least(&p.embedding, EMBEDDING_MIN_BYTES);
        assert_eq!(present, expected);
    }

    #[test]
    fn progress_label_includes_byte_counts_in_mb() {
        assert_eq!(
            progress_label(DownloadStage::Embedding, 12_582_912, 105_906_176),
            "downloading embedding model · 12.0 MB / 101.0 MB"
        );
    }

    #[test]
    fn progress_label_is_stage_only_when_total_unknown() {
        assert_eq!(
            progress_label(DownloadStage::Extracting, 0, 0),
            "extracting model"
        );
    }

    #[test]
    fn check_download_complete_ok_when_total_unknown() {
        assert!(check_download_complete("u", 0, 0).is_ok());
        assert!(check_download_complete("u", 42, 0).is_ok());
    }

    #[test]
    fn check_download_complete_ok_when_full() {
        assert!(check_download_complete("u", 100, 100).is_ok());
    }

    #[test]
    fn check_download_complete_errs_on_short_read() {
        let err = check_download_complete("u", 50, 100).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("50"),
            "message should name actual bytes: {msg}"
        );
        assert!(
            msg.contains("100"),
            "message should name expected bytes: {msg}"
        );
    }

    /// specs/0061 W2 fix round 2 (R20): a gzip-compressed body means
    /// `Content-Length` is the compressed size while `buf.len()` holds
    /// decompressed bytes, so `downloaded > total` is legitimate — must never
    /// be flagged as truncated (a `!=` check would spuriously fail this).
    #[test]
    fn check_download_complete_ok_when_downloaded_exceeds_total() {
        assert!(check_download_complete("u", 150, 100).is_ok());
    }

    #[test]
    fn extract_finds_model_onnx_in_tarball() {
        // Build a tiny in-memory .tar.bz2 containing `pkg/model.onnx` and confirm
        // we extract exactly its bytes — verifies the tar+bzip2 path without a
        // network download.
        let payload = b"FAKE_ONNX_BYTES_FOR_TEST";
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "pkg/model.onnx", &payload[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut bz = Vec::new();
        {
            let mut enc = bzip2::write::BzEncoder::new(&mut bz, bzip2::Compression::fast());
            enc.write_all(&tar_buf).unwrap();
            enc.finish().unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("segmentation.onnx");
        extract_segmentation_onnx(&bz, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
    }
}
