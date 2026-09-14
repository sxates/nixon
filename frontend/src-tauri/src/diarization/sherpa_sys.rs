//! Direct `sherpa-rs-sys` binding for the offline diarizer, so ONNX
//! `num_threads` is settable (specs/0053 W2).
//!
//! `sherpa_rs::diarize::Diarize` hardcodes `num_threads: 1` on both the
//! segmentation and embedding model configs and exposes no override, pinning
//! the whole post-meeting pass to one core. This module mirrors sherpa-rs
//! 0.6.8's construction and compute sequence exactly and changes only that
//! integer.
//!
//! All `unsafe` for W2 is contained here. `sherpa-rs` remains a dependency —
//! `crate::diarization::identity` uses its standalone `EmbeddingExtractor`,
//! which already exposes `num_threads`.
//!
//! `crate::diarization::sherpa::SherpaDiarizer` is wired over to this module
//! (specs/0053 W3), so its public surface is live.

use std::ffi::{c_void, CString};
use std::path::Path;
use std::ptr::null_mut;

use anyhow::{anyhow, Result};

/// One diarized turn, identical in shape to `sherpa_rs::diarize::Segment` so
/// call sites are unchanged.
#[derive(Debug, Clone)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub speaker: i32,
}

/// `(processed_chunks, total_chunks) -> 0 to continue`. Matches sherpa's C
/// callback contract; we always continue.
pub type ProgressCallback = Box<dyn Fn(i32, i32) -> i32 + Send + 'static>;

/// Construction knobs. Mirrors `sherpa_rs::diarize::DiarizeConfig` plus the
/// `num_threads` it does not expose.
#[derive(Debug, Clone)]
pub struct ThreadedDiarizeConfig {
    pub num_clusters: i32,
    pub threshold: f32,
    pub min_duration_on: f32,
    pub min_duration_off: f32,
    pub provider: String,
    pub num_threads: i32,
}

impl ThreadedDiarizeConfig {
    /// Build with `num_threads` from [`crate::diarization::accel::diarizer_threads`].
    pub fn with_defaults(
        num_clusters: i32,
        threshold: f32,
        min_duration_on: f32,
        min_duration_off: f32,
        provider: String,
    ) -> Self {
        Self {
            num_clusters,
            threshold,
            min_duration_on,
            min_duration_off,
            provider,
            num_threads: crate::diarization::accel::diarizer_threads() as i32,
        }
    }
}

/// Owns the C diarizer handle.
#[derive(Debug)]
pub struct ThreadedDiarize {
    sd: *const sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarization,
}

// SAFETY: the handle is only ever used behind the `Mutex` in
// `crate::diarization::sherpa::SherpaDiarizer`, which serializes all access.
// sherpa's C API has no thread-affinity requirement on the handle itself.
unsafe impl Send for ThreadedDiarize {}

impl ThreadedDiarize {
    pub fn new(
        segmentation_model: &Path,
        embedding_model: &Path,
        config: ThreadedDiarizeConfig,
    ) -> Result<Self> {
        if !segmentation_model.exists() {
            return Err(anyhow!(
                "segmentation model not found: {}",
                segmentation_model.display()
            ));
        }
        if !embedding_model.exists() {
            return Err(anyhow!(
                "embedding model not found: {}",
                embedding_model.display()
            ));
        }

        let segmentation = path_to_cstring(segmentation_model)?;
        let embedding = path_to_cstring(embedding_model)?;
        let provider = CString::new(config.provider.as_str())
            .map_err(|e| anyhow!("provider string contains an interior NUL: {e}"))?;

        // SAFETY: `segmentation`, `embedding` and `provider` are named locals
        // that outlive the create call below, so the pointers stay valid for
        // the whole of it. sherpa copies what it needs during construction.
        let sd = unsafe {
            let c_config = sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarizationConfig {
                embedding: sherpa_rs_sys::SherpaOnnxSpeakerEmbeddingExtractorConfig {
                    model: embedding.as_ptr(),
                    // THE ENTIRE POINT OF THIS MODULE. sherpa-rs hardcodes 1.
                    num_threads: config.num_threads,
                    debug: 0,
                    provider: provider.as_ptr(),
                },
                clustering: sherpa_rs_sys::SherpaOnnxFastClusteringConfig {
                    num_clusters: config.num_clusters,
                    threshold: config.threshold,
                },
                min_duration_off: config.min_duration_off,
                min_duration_on: config.min_duration_on,
                segmentation: sherpa_rs_sys::SherpaOnnxOfflineSpeakerSegmentationModelConfig {
                    pyannote:
                        sherpa_rs_sys::SherpaOnnxOfflineSpeakerSegmentationPyannoteModelConfig {
                            model: segmentation.as_ptr(),
                        },
                    // ...and here.
                    num_threads: config.num_threads,
                    debug: 0,
                    provider: provider.as_ptr(),
                },
            };
            sherpa_rs_sys::SherpaOnnxCreateOfflineSpeakerDiarization(&c_config)
        };

        if sd.is_null() {
            return Err(anyhow!("Failed to initialize offline speaker diarization"));
        }

        log::info!(
            "diarization: sherpa diarizer initialized on '{}' with {} ONNX thread(s)",
            config.provider,
            config.num_threads
        );

        Ok(Self { sd })
    }

    /// Run diarization over 16 kHz mono f32 samples.
    ///
    /// The error text for the empty-result case is preserved verbatim from
    /// sherpa-rs ("No segments found or invalid pointer."), because
    /// `crate::diarization::sherpa` matches on `NO_SEGMENTS_MARKER` to tell
    /// "no speech" apart from a genuine model failure.
    pub fn compute(
        &mut self,
        mut samples: Vec<f32>,
        progress_callback: Option<ProgressCallback>,
    ) -> Result<Vec<Segment>> {
        let samples_ptr = samples.as_mut_ptr();
        let samples_len = samples.len() as i32;
        let mut segments = Vec::new();

        // SAFETY: `samples` is alive for the whole call; the callback box (when
        // present) is alive for the whole call and sherpa invokes it inline,
        // synchronously, never storing it past its own return. `result` and
        // (when non-null) `segments_ptr` are both sherpa heap allocations
        // (mirrors upstream `sherpa_rs::diarize::Diarize::compute`) that the
        // caller must free — `SortByStartTime` allocates a fresh array on
        // every call, so this leaks on every run if skipped. Both destroy
        // calls run on every exit path, including "no segments found": unlike
        // upstream (whose early `bail!` on that path skips them, leaking
        // `result`), we defer the error return until after both are freed.
        // `DestroySegment`/`DestroyResult` are plain C++ `delete[]`/`delete`
        // (see sherpa-onnx c-api.cc), safe to call with a null pointer.
        unsafe {
            let mut callback_box = progress_callback.map(Box::new);
            let callback_ptr = callback_box
                .as_mut()
                .map(|b| b.as_mut() as *mut ProgressCallback as *mut c_void)
                .unwrap_or(null_mut());

            let result = sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarizationProcessWithCallback(
                self.sd,
                samples_ptr,
                samples_len,
                if callback_box.is_some() {
                    Some(progress_trampoline)
                } else {
                    None
                },
                callback_ptr,
            );

            let num_segments =
                sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarizationResultGetNumSegments(result);
            let segments_ptr =
                sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarizationResultSortByStartTime(result);

            let outcome = if segments_ptr.is_null() || num_segments <= 0 {
                Err(anyhow!("No segments found or invalid pointer."))
            } else {
                let slice = std::slice::from_raw_parts(segments_ptr, num_segments as usize);
                for s in slice {
                    segments.push(Segment {
                        start: s.start,
                        end: s.end,
                        speaker: s.speaker,
                    });
                }
                Ok(())
            };

            // Mirrors upstream's destroy ordering exactly (segment array before
            // the result it was sorted from), but — unlike upstream — runs on
            // every exit path, not just the success path.
            sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarizationDestroySegment(segments_ptr);
            sherpa_rs_sys::SherpaOnnxOfflineSpeakerDiarizationDestroyResult(result);

            outcome?;
        }

        Ok(segments)
    }
}

impl Drop for ThreadedDiarize {
    fn drop(&mut self) {
        // SAFETY: `sd` was produced by SherpaOnnxCreateOfflineSpeakerDiarization
        // and is non-null (checked in `new`); this runs exactly once.
        unsafe {
            sherpa_rs_sys::SherpaOnnxDestroyOfflineSpeakerDiarization(self.sd);
        }
    }
}

/// C ABI shim that forwards sherpa's progress ticks to the boxed Rust closure.
///
/// # Safety
/// `arg` must be the `*mut ProgressCallback` handed to
/// `SherpaOnnxOfflineSpeakerDiarizationProcessWithCallback`, still alive.
unsafe extern "C" fn progress_trampoline(
    num_processed_chunks: i32,
    num_total_chunks: i32,
    arg: *mut c_void,
) -> i32 {
    if arg.is_null() {
        return 0;
    }
    let callback = &*(arg as *const ProgressCallback);
    callback(num_processed_chunks, num_total_chunks)
}

/// Bridge a borrowed `FnMut(f32)` fraction sink to sherpa's owned
/// `Fn(i32, i32) -> i32 + Send + 'static` chunk callback.
///
/// # Safety
/// The referent of `sink` must outlive the `compute` call this callback is
/// passed to. That holds because sherpa invokes the callback **synchronously,
/// inline on the calling thread** and never stores it past its own return
/// (verified in sherpa-rs 0.6.8 `diarize.rs`), and `compute` blocks until done.
pub unsafe fn fraction_progress_callback(
    sink: *mut (dyn FnMut(f32) + '_),
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    last_total: std::sync::Arc<std::sync::atomic::AtomicI32>,
) -> ProgressCallback {
    struct SendPtr(*mut (dyn FnMut(f32) + 'static));
    // SAFETY: only dereferenced on the thread that called `compute` — sherpa
    // runs the callback inline and synchronously — so the assertion is never
    // exercised across threads.
    unsafe impl Send for SendPtr {}

    let sink: *mut (dyn FnMut(f32) + 'static) = std::mem::transmute(sink);
    let send_ptr = SendPtr(sink);

    Box::new(move |processed: i32, total: i32| -> i32 {
        // Capture the whole SendPtr (not the raw field) so the closure stays
        // Send under disjoint-closure-capture rules.
        let send_ptr = &send_ptr;
        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        last_total.store(total, std::sync::atomic::Ordering::Relaxed);
        if total > 0 {
            let frac = (processed as f32 / total as f32).clamp(0.0, 1.0);
            // SAFETY: see the function-level note.
            unsafe { (*send_ptr.0)(frac) };
        }
        0
    })
}

fn path_to_cstring(path: &Path) -> Result<CString> {
    let s = path
        .to_str()
        .ok_or_else(|| anyhow!("model path is not valid UTF-8: {}", path.display()))?;
    CString::new(s).map_err(|e| anyhow!("model path contains an interior NUL: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of this module. If this ever reads 1, the wrapper has
    /// silently regressed to sherpa-rs's behaviour and long meetings crawl again.
    #[test]
    fn config_carries_more_than_one_thread_by_default() {
        let config = ThreadedDiarizeConfig::with_defaults(0, 0.8, 0.3, 0.5, "cpu".to_string());
        assert!(
            config.num_threads > 1
                || std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1) == 1,
            "diarizer must not be pinned to one thread; got {}",
            config.num_threads
        );
    }

    /// A missing model file must be a clean anyhow::Err, never a null-pointer
    /// deref or a panic across the FFI boundary.
    #[test]
    fn missing_model_files_error_cleanly() {
        let missing = std::path::Path::new("/nonexistent/model.onnx");
        let err = ThreadedDiarize::new(
            missing,
            missing,
            ThreadedDiarizeConfig::with_defaults(0, 0.8, 0.3, 0.5, "cpu".to_string()),
        )
        .expect_err("missing models must error");
        assert!(format!("{err:#}").contains("not found"), "got: {err:#}");
    }
}
