//! Per-cluster speaker embeddings + the pure vector math the live stable-id
//! registry and (later) cross-meeting identity matching are built on. (specs/0011)
//!
//! ## Embedding-extraction spike outcome (sherpa-rs 0.6.8)
//!
//! The OPEN QUESTION in `specs/0011` was: does `sherpa_rs::diarize::Diarize`
//! expose the per-cluster centroid embeddings its clustering already computed?
//!
//! **It does not.** `Diarize::compute()` returns only
//! `Segment { start, end, speaker: i32 }` (see `sherpa-rs-0.6.8/src/diarize.rs`),
//! and the underlying C handle (`SherpaOnnxOfflineSpeakerDiarization`) offers no
//! "get cluster centroid / embedding" entry point. The centroids live entirely
//! inside the C++ fast-clustering pass and are dropped.
//!
//! So we take the **sanctioned re-embed fallback**: re-run the *already-downloaded*
//! embedding ONNX (`nemo_en_titanet_large.onnx`, specs/0043 W2.2) over each
//! cluster's pooled representative speech via `sherpa_rs::speaker_id::
//! EmbeddingExtractor` (a public API loading the very same model), then mean-pool
//! and L2-normalize. This is fully in our control and reuses the model the
//! diarizer already requires — no new model, no new download. See
//! [`ClusterEmbedder`].
//!
//! Everything below the extractor is **pure** (`l2_normalize`, `cosine_similarity`,
//! `embedding_to_bytes`/`embedding_from_bytes`) so it unit-tests without any model
//! or audio, and is shared by `live.rs` (the session stable-id registry) now and
//! by the future cross-meeting `identity.rs` / the `speakers.embedding` BLOB.

use anyhow::{anyhow, Result};

use crate::diarization::SpeakerTurn;

/// Sample rate the embedding model expects (same as the diarizer).
const EMBEDDING_SAMPLE_RATE: u32 = 16_000;

/// Identifier of the embedding model that produced a stored vector, so a
/// model swap can't silently compare incomparable vectors (cosine across models
/// is meaningless). Persisted alongside the BLOB in a later stage; defined here
/// because this module owns the embedding contract.
///
/// specs/0043 W2.2: bumped from `3dspeaker_campplus_sv_en_voxceleb_16k` when the
/// embedding model became TitaNet-L. Voiceprint storage is keyed per model id
/// (ADR-0007 §4), so old CAM++ voiceprints stop matching and galleries repopulate
/// organically under this id — dual-keyed re-enrollment, no migration (ADR-0011).
pub const EMBEDDING_MODEL_ID: &str = "nemo_en_titanet_large";

// ---------------------------------------------------------------------------
// Pure vector math (no model, no audio — unit-tested unconditionally).
// ---------------------------------------------------------------------------

/// Return an L2-normalized (unit-length) copy of `v`. A zero (or all-zero) vector
/// is returned unchanged — there is no meaningful direction to normalize to, and
/// dividing by zero would yield NaNs that poison every later cosine.
pub fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return v.to_vec();
    }
    v.iter().map(|x| x / norm).collect()
}

/// Cosine similarity of two equal-length vectors, in `[-1.0, 1.0]`.
///
/// Returns `0.0` (orthogonal / "no information") for a length mismatch or a
/// zero-magnitude operand rather than erroring or NaN-ing — callers treat the
/// score as a similarity and a zero score simply never matches a positive
/// threshold, which is the safe behavior for a degenerate input.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f32::EPSILON {
        return 0.0;
    }
    (dot / denom).clamp(-1.0, 1.0)
}

/// Running mean of two embeddings weighted by their sample counts, returned
/// L2-normalized. Used by the session registry to fold a fresh cluster centroid
/// into an existing session speaker's representative vector.
///
/// `acc`/`acc_n` is the accumulated (already-normalized) vector and how many
/// observations it represents; `new` is the fresh (normalized) centroid for `1`
/// observation by default — pass `new_n > 1` to weight a pooled centroid more.
pub fn weighted_mean(acc: &[f32], acc_n: u32, new: &[f32], new_n: u32) -> Vec<f32> {
    if acc.len() != new.len() || acc.is_empty() {
        // Degenerate; prefer whichever side has data.
        return if acc.is_empty() {
            l2_normalize(new)
        } else {
            l2_normalize(acc)
        };
    }
    let aw = acc_n.max(1) as f32;
    let nw = new_n.max(1) as f32;
    let total = aw + nw;
    let mean: Vec<f32> = acc
        .iter()
        .zip(new.iter())
        .map(|(a, b)| (a * aw + b * nw) / total)
        .collect();
    l2_normalize(&mean)
}

/// Serialize an embedding to little-endian f32 bytes for the `speakers.embedding`
/// BLOB column. Little-endian is fixed regardless of host so a DB copied between
/// machines stays readable (matches every other LE serialization in this crate).
pub fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Deserialize a little-endian f32 BLOB back into an embedding. Errors if the
/// byte length is not a multiple of 4 (a corrupt/foreign BLOB) rather than
/// silently truncating.
pub fn embedding_from_bytes(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return Err(anyhow!(
            "embedding BLOB length {} is not a multiple of 4 bytes",
            bytes.len()
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

// ---------------------------------------------------------------------------
// Per-cluster embedding extraction (the re-embed fallback).
// ---------------------------------------------------------------------------

/// Total seconds of pooled speech, per cluster, fed to the embedding model. CAM++
/// produces a stable utterance-level vector from a few seconds; we cap so a very
/// talkative cluster doesn't pay an unbounded re-embed cost (live runs repeatedly).
const MAX_POOL_SECONDS: f32 = 8.0;
/// Skip embedding a cluster with less than this much pooled speech — too little
/// audio yields a noisy, untrustworthy vector (better no embedding than a bad one).
const MIN_POOL_SECONDS: f32 = 0.5;

/// Wraps the loaded embedding ONNX (via `sherpa_rs::speaker_id::EmbeddingExtractor`)
/// to compute one representative L2-normalized embedding per diarization cluster by
/// pooling that cluster's speech out of the same 16 kHz mono buffer the diarizer
/// ran on. Runs on the preferred provider (CoreML on macOS) with a CPU fallback
/// and multiple intra-op threads (see [`crate::diarization::accel`]).
///
/// `!Sync` underneath (`compute_speaker_embedding(&mut self, …)`), so callers that
/// need shared access guard it; `live.rs` owns it on its blocking pass thread, so
/// no extra locking is needed there.
pub struct ClusterEmbedder {
    extractor: sherpa_rs::speaker_id::EmbeddingExtractor,
}

impl ClusterEmbedder {
    /// Construct from the embedding model path (the same `3dspeaker_campplus…onnx`
    /// the diarizer loads).
    ///
    /// Uses the preferred ONNX execution provider (CoreML on macOS) with a
    /// guaranteed CPU fallback, and multiple intra-op threads — unlike the
    /// diarizer, `EmbeddingExtractor` exposes `num_threads`, so the per-cluster
    /// re-embed pass parallelizes. Both knobs are env-overridable via
    /// [`crate::diarization::accel`]. Same model/dimensions either way, so vectors
    /// stay comparable.
    pub fn new(embedding_model: &std::path::Path) -> Result<Self> {
        let model = embedding_model.to_string_lossy().into_owned();
        let num_threads = Some(crate::diarization::accel::embedding_threads());
        let extractor = crate::diarization::accel::build_with_provider_fallback(
            "embedding extractor",
            |provider| {
                sherpa_rs::speaker_id::EmbeddingExtractor::new(
                    sherpa_rs::speaker_id::ExtractorConfig {
                        model: model.clone(),
                        provider: Some(provider.to_string()),
                        num_threads,
                        debug: false,
                    },
                )
                .map_err(|e| anyhow!("failed to init embedding extractor: {e}"))
            },
        )?;
        Ok(Self { extractor })
    }

    /// Dimension of the vectors this extractor produces.
    pub fn dim(&self) -> usize {
        self.extractor.embedding_size
    }

    /// Compute one L2-normalized embedding per *remote* cluster (`spk_N`) present in
    /// `turns`, by concatenating up to [`MAX_POOL_SECONDS`] of that cluster's speech
    /// from `samples_16k_mono` and running the embedding model once over it.
    ///
    /// - `samples_16k_mono` is the exact buffer the diarizer clustered (same time base).
    /// - The local/mic key is never present in system-channel `turns` (mic isn't
    ///   diarized), so no owner voiceprinting happens here by construction.
    /// - A cluster with < [`MIN_POOL_SECONDS`] pooled speech is skipped (omitted from
    ///   the map) rather than embedded badly.
    ///
    /// Returns `speaker_key -> embedding`. Best-effort per cluster: a single
    /// cluster's extraction failure is logged and skipped, not propagated, so one
    /// bad cluster never sinks the whole pass.
    pub fn embeddings_for_turns(
        &mut self,
        turns: &[SpeakerTurn],
        samples_16k_mono: &[f32],
    ) -> std::collections::HashMap<String, Vec<f32>> {
        use std::collections::BTreeMap;

        // Group turns by speaker key, in stable order, accumulating pooled samples
        // up to the cap. BTreeMap keeps the iteration deterministic for tests/logs.
        let mut pooled: BTreeMap<String, Vec<f32>> = BTreeMap::new();
        let max_samples = (MAX_POOL_SECONDS * EMBEDDING_SAMPLE_RATE as f32) as usize;

        for t in turns {
            let entry = pooled.entry(t.speaker.clone()).or_default();
            if entry.len() >= max_samples {
                continue;
            }
            let start = (t.start.max(0.0) * EMBEDDING_SAMPLE_RATE as f32) as usize;
            let end = (t.end.max(0.0) * EMBEDDING_SAMPLE_RATE as f32) as usize;
            let (start, end) = (
                start.min(samples_16k_mono.len()),
                end.min(samples_16k_mono.len()),
            );
            if end <= start {
                continue;
            }
            let want = (max_samples - entry.len()).min(end - start);
            entry.extend_from_slice(&samples_16k_mono[start..start + want]);
        }

        let min_samples = (MIN_POOL_SECONDS * EMBEDDING_SAMPLE_RATE as f32) as usize;
        let mut out = std::collections::HashMap::new();
        for (key, samples) in pooled {
            if samples.len() < min_samples {
                log::debug!(
                    "diarization: skipping embedding for {key} — only {} pooled samples (< {min_samples})",
                    samples.len()
                );
                continue;
            }
            match self
                .extractor
                .compute_speaker_embedding(samples, EMBEDDING_SAMPLE_RATE)
            {
                Ok(v) => {
                    out.insert(key, l2_normalize(&v));
                }
                Err(e) => {
                    log::warn!(
                        "diarization: embedding extraction for {key} failed: {e} — skipping"
                    );
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_yields_unit_length() {
        let v = vec![3.0, 4.0]; // norm 5
        let n = l2_normalize(&v);
        assert!((n[0] - 0.6).abs() < 1e-6);
        assert!((n[1] - 0.8).abs() < 1e-6);
        let len = (n[0] * n[0] + n[1] * n[1]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_zero_vector_is_unchanged_not_nan() {
        let v = vec![0.0f32; 4];
        let n = l2_normalize(&v);
        assert_eq!(n, v);
        assert!(n.iter().all(|x| !x.is_nan()));
    }

    #[test]
    fn cosine_identical_is_one() {
        let a = l2_normalize(&[1.0, 2.0, 3.0]);
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_is_minus_one() {
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_handles_length_mismatch_and_zero() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_is_unaffected_by_magnitude() {
        // Cosine compares direction, not magnitude.
        let a = [1.0, 1.0, 0.0];
        let b = [5.0, 5.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn byte_roundtrip_is_exact() {
        let v = vec![0.0, 1.0, -1.5, 3.5, -2.25, 1e-7, 1e7];
        let bytes = embedding_to_bytes(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        let back = embedding_from_bytes(&bytes).unwrap();
        assert_eq!(back, v); // exact: f32 LE roundtrip is lossless
    }

    #[test]
    fn from_bytes_rejects_misaligned_length() {
        assert!(embedding_from_bytes(&[0u8; 5]).is_err());
        assert!(embedding_from_bytes(&[0u8; 7]).is_err());
        assert!(embedding_from_bytes(&[]).unwrap().is_empty());
    }

    #[test]
    fn weighted_mean_moves_toward_new_proportionally() {
        let a = l2_normalize(&[1.0, 0.0]);
        let b = l2_normalize(&[0.0, 1.0]);
        // Equal weight → 45°, both components equal after normalize.
        let m = weighted_mean(&a, 1, &b, 1);
        assert!((m[0] - m[1]).abs() < 1e-6);
        // Heavily weight `a` → stays close to `a`.
        let m2 = weighted_mean(&a, 100, &b, 1);
        assert!(m2[0] > 0.99);
    }
}
