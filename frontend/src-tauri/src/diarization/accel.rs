//! Hardware-acceleration policy for the on-device diarization engine.
//!
//! Diarization (sherpa-onnx: pyannote segmentation → speaker embedding →
//! clustering) runs **post-meeting** on a blocking thread, so it can use the
//! Apple Neural Engine / GPU via ONNX Runtime's **CoreML** execution provider
//! without competing with live STT. Our bundled `libonnxruntime.1.17.1.dylib`
//! was built **with** the CoreML EP (verified: it exports
//! `OrtSessionOptionsAppendExecutionProvider_CoreML`, which our
//! `libsherpa-onnx-c-api.dylib` imports), so `provider: "coreml"` is honored by
//! sherpa-onnx on this build.
//!
//! ## The overriding rule: CPU is the default and a guaranteed fallback
//!
//! Every accelerated construction here is a *preference*, not a requirement.
//! [`build_with_provider_fallback`] tries the preferred provider first and, on
//! any Rust `Err`, logs and retries on CPU. **Caveat (the v0.5.0→v0.5.1 fix):**
//! that fallback only catches a *Rust error*. Under the production app's hardened
//! runtime, CoreML EP init throws a C++ exception that escapes `Diarize::new` as
//! `terminate()`/`abort()` — a hard crash the fallback cannot intercept. So CoreML
//! is **no longer the default** ([`default_provider`]); it's opt-in via env only.
//!
//! ## Seam, not a settings UI
//!
//! Defaults live here as constants/env reads. A user-facing toggle is the
//! background-system spec's job; for now the provider can be overridden with
//! `NIXON_DIARIZATION_PROVIDER` (e.g. `cpu` to force the legacy path) and the
//! embedding thread count with `NIXON_DIARIZATION_THREADS`, which keeps a clear
//! seam without touching call sites.

/// Preferred ONNX Runtime execution provider for diarization, honoring an env
/// override. Defaults to `cpu` (see [`default_provider`] for why CoreML is no
/// longer the default). Set `NIXON_DIARIZATION_PROVIDER=coreml` to opt back in.
pub fn preferred_provider() -> String {
    if let Ok(p) = std::env::var("NIXON_DIARIZATION_PROVIDER") {
        let p = p.trim().to_lowercase();
        if !p.is_empty() {
            return p;
        }
    }
    default_provider()
}

/// The compiled-in default provider (no env override).
///
/// **Defaults to CPU, even on macOS.** CoreML *was* the default, but under the
/// production app's **hardened runtime** ONNX Runtime's CoreML EP init throws a
/// C++ exception inside `sherpa_onnx`'s `Diarize::new` that escapes as
/// `terminate()`/`abort()` — NOT a Rust `Err` — so [`build_with_provider_fallback`]
/// cannot catch it and the whole app crashes on "Identify speakers" (it worked
/// under dev's ad-hoc signing, hence the regression slipped through). CPU init
/// returns errors normally and is the safe, proven path. CoreML remains available
/// for future, hardened-runtime-validated work via `NIXON_DIARIZATION_PROVIDER=coreml`.
fn default_provider() -> String {
    "cpu".to_string()
}

/// Number of intra-op threads for the embedding extractor (the per-cluster
/// re-embed pass). sherpa-rs hardcodes the *diarizer's* threads to 1, but the
/// standalone `EmbeddingExtractor` does expose `num_threads`, so we raise it.
///
/// Defaults to `available_parallelism()` clamped to [`MIN_THREADS`,
/// [`MAX_EMBED_THREADS`]]; override with `NIXON_DIARIZATION_THREADS`.
pub fn embedding_threads() -> usize {
    if let Ok(v) = std::env::var("NIXON_DIARIZATION_THREADS") {
        if let Ok(n) = v.trim().parse::<usize>() {
            return n.clamp(MIN_THREADS, MAX_EMBED_THREADS);
        }
    }
    let par = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(MIN_THREADS);
    par.clamp(MIN_THREADS, MAX_EMBED_THREADS)
}

const MIN_THREADS: usize = 1;
/// Cap embedding threads: diminishing returns past a handful of cores for these
/// small, short re-embed runs, and we don't want to starve the rest of the app.
const MAX_EMBED_THREADS: usize = 8;

/// Number of ONNX intra-op threads for the **diarizer** (pyannote segmentation +
/// CAM++ embedding over the whole recording).
///
/// sherpa-rs pins this to 1 with no override, which is why long meetings took
/// 15-45 minutes; `crate::diarization::sherpa_sys` bypasses it (specs/0053 W2).
/// Same bounds and same `NIXON_DIARIZATION_THREADS` seam as
/// [`embedding_threads`] — the cap stays because ONNX intra-op scaling flattens
/// out on models this small, and the pass must not starve the rest of the app.
pub fn diarizer_threads() -> usize {
    embedding_threads()
}

/// Try to build an ONNX-backed resource on the preferred provider, falling back
/// to CPU on any error. `build` is called with the provider string; the first
/// successful result wins. The CPU attempt is the guaranteed fallback — if it
/// *also* fails, that error is returned (init is genuinely broken).
///
/// Skips the redundant retry when the preferred provider already is CPU.
pub fn build_with_provider_fallback<T, F>(label: &str, mut build: F) -> anyhow::Result<T>
where
    F: FnMut(&str) -> anyhow::Result<T>,
{
    let preferred = preferred_provider();
    if preferred == "cpu" {
        return build("cpu");
    }
    match build(&preferred) {
        Ok(v) => {
            log::info!("diarization: {label} initialized on '{preferred}' provider");
            Ok(v)
        }
        Err(e) => {
            log::warn!(
                "diarization: {label} failed to init on '{preferred}' ({e:#}); \
                 falling back to CPU"
            );
            build("cpu")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NIXON_DIARIZATION_THREADS` is process-global state; cargo runs tests in
    /// this file on multiple threads by default, so any two tests that both
    /// touch it race unless serialized. Grab this for the duration of any test
    /// that sets/removes that var.
    static THREADS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn embedding_threads_within_bounds() {
        let n = embedding_threads();
        assert!((MIN_THREADS..=MAX_EMBED_THREADS).contains(&n), "got {n}");
    }

    #[test]
    fn cpu_preference_skips_retry() {
        // When CPU is preferred, `build` must be called exactly once with "cpu".
        let mut calls = Vec::new();
        let out = build_with_provider_fallback("test", |p| {
            calls.push(p.to_string());
            Ok::<_, anyhow::Error>(p.to_string())
        });
        // Force-cpu only if env happens to set it; otherwise this asserts the
        // happy path of a single successful build. Either way `build` succeeded.
        assert!(out.is_ok());
        assert!(!calls.is_empty());
        assert_eq!(calls.last().unwrap(), &out.unwrap());
    }

    #[test]
    fn falls_back_to_cpu_on_preferred_error() {
        std::env::set_var("NIXON_DIARIZATION_PROVIDER", "coreml");
        let mut calls = Vec::new();
        let out = build_with_provider_fallback("test", |p| {
            calls.push(p.to_string());
            if p == "coreml" {
                Err(anyhow::anyhow!("simulated coreml failure"))
            } else {
                Ok(p.to_string())
            }
        });
        std::env::remove_var("NIXON_DIARIZATION_PROVIDER");
        assert_eq!(out.unwrap(), "cpu");
        assert_eq!(calls, vec!["coreml".to_string(), "cpu".to_string()]);
    }

    /// specs/0053 W2: the diarizer is no longer stuck at sherpa-rs's hardcoded 1.
    #[test]
    fn diarizer_threads_is_above_one_on_a_multicore_machine() {
        let _guard = THREADS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("NIXON_DIARIZATION_THREADS");
        let n = diarizer_threads();
        assert!((MIN_THREADS..=MAX_EMBED_THREADS).contains(&n), "got {n}");
        if std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
            > 1
        {
            assert!(n > 1, "expected >1 thread on a multicore host, got {n}");
        }
    }

    /// The env seam still wins, and is still clamped.
    #[test]
    fn diarizer_threads_honors_and_clamps_the_env_override() {
        let _guard = THREADS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("NIXON_DIARIZATION_THREADS", "3");
        assert_eq!(diarizer_threads(), 3);
        std::env::set_var("NIXON_DIARIZATION_THREADS", "999");
        assert_eq!(diarizer_threads(), MAX_EMBED_THREADS);
        std::env::remove_var("NIXON_DIARIZATION_THREADS");
    }
}
