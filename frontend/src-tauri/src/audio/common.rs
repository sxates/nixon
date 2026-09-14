use crate::parakeet_engine::WordStamp;
use crate::transcripts::TranscriptSegment;
use anyhow::Result;
use log::{debug, info};
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

/// Which capture channel dominated a stretch of the MIXED stream (specs/0029 WS3.4).
///
/// The pipeline mixes mic + system before STT, destroying channel identity; this tag
/// is computed from per-window RMS dominance of the two pre-mix tracks and threaded
/// through to the transcript segment so the offline diarization pass can keep
/// mic-tagged segments attributed to "You" unconditionally. Wire/DB values are the
/// device-naming convention strings ("microphone"/"system" — never "input"/"output").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelTag {
    /// The local user's microphone track clearly dominated.
    Microphone,
    /// The remote participants' system-audio track clearly dominated.
    System,
    /// Both tracks were active with no clear dominance (overlapped speech).
    Mixed,
}

impl ChannelTag {
    /// The wire/DB string for this tag (`transcripts.channel` values).
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelTag::Microphone => "microphone",
            ChannelTag::System => "system",
            ChannelTag::Mixed => "mixed",
        }
    }
}

/// One contiguous stretch of a VAD segment that shares a single [`ChannelTag`]
/// (specs/0055). Times are recording-relative **seconds**, matching
/// `audio_start_time`/`audio_end_time` on the transcript wire.
///
/// Where [`ChannelTag`] answers "who dominated this row?", a run list answers
/// "who was speaking *when* inside it" — the 600 ms signal that
/// `dominant_channel_for_span` collapses. 32% of real rows straddle an
/// owner<->remote handoff (specs/0055), so the single tag necessarily mislabels
/// one side of those rows. Runs tile their span contiguously, so text can be
/// apportioned across them without holes.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChannelRun {
    /// Recording-relative start (seconds).
    #[serde(rename = "s")]
    pub start: f64,
    /// Recording-relative end (seconds).
    #[serde(rename = "e")]
    pub end: f64,
    /// Which channel dominated this stretch.
    #[serde(rename = "c")]
    pub tag: ChannelTag,
}

/// One VAD speech segment bound for the transcription worker, carrying its
/// capture-channel attribution (specs/0029 WS3.4). `channel` is `None` when the
/// pipeline had no channel evidence for the segment's span (e.g. classification
/// history exhausted) — persisted as NULL, same as legacy rows.
#[derive(Debug, Clone)]
pub struct TranscriptionChunk {
    pub chunk: crate::audio::recording_state::AudioChunk,
    pub channel: Option<ChannelTag>,
    /// specs/0055: the same evidence at 600 ms resolution, so a row straddling a
    /// speaker handoff can be split instead of taking one label. Empty when the
    /// pipeline had no classified windows for the span (same condition as
    /// `channel == None`).
    pub channel_runs: Vec<ChannelRun>,
}

static ENGINE_LIFECYCLE_LOCK: Lazy<Arc<AsyncMutex<()>>> =
    Lazy::new(|| Arc::new(AsyncMutex::new(())));

pub(crate) async fn acquire_engine_lifecycle_lock() -> OwnedMutexGuard<()> {
    ENGINE_LIFECYCLE_LOCK.clone().lock_owned().await
}

/// Unload the transcription engine after a batch job (import or retranscription).
/// Skips unloading if a live recording is currently in progress, since recording
/// uses the same global engine instances.
pub(crate) async fn unload_engine_after_batch(use_parakeet: bool) {
    let _engine_lifecycle_guard = acquire_engine_lifecycle_lock().await;

    if crate::audio::recording_commands::is_recording().await {
        log::info!("Skipping model unload after batch: recording in progress");
        return;
    }

    if use_parakeet {
        use crate::parakeet_engine::commands::PARAKEET_ENGINE;
        let engine = {
            let guard = PARAKEET_ENGINE.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().cloned()
        };
        if let Some(e) = engine {
            e.unload_model().await;
        }
    } else {
        use crate::whisper_engine::commands::WHISPER_ENGINE;
        let engine = {
            let guard = WHISPER_ENGINE.lock().unwrap_or_else(|e| e.into_inner());
            guard.as_ref().cloned()
        };
        if let Some(e) = engine {
            e.unload_model().await;
        }
    }
}

/// Create transcript segments from transcription results.
/// Each tuple is (text, start_ms, end_ms) from VAD timestamps.
pub(crate) fn create_transcript_segments(
    transcripts: &[(String, f64, f64)],
) -> Vec<TranscriptSegment> {
    create_transcript_segments_with_words(transcripts, &[])
}

/// Like [`create_transcript_segments`], but also threads each segment's
/// optional per-word timestamps (specs/0046 WS2) through to
/// `TranscriptSegment.word_timestamps`, serialized as JSON. `words[i]`
/// corresponds to `transcripts[i]`; a shorter/absent `words` (e.g. the
/// word-less Whisper batch path) leaves the remaining segments' tag `None` —
/// same as [`create_transcript_segments`].
///
/// Parakeet's per-word `start`/`end` are SEGMENT-relative (0-based within the
/// buffer passed to `transcribe_audio_timestamped`), but every other stored
/// timestamp (`audio_start_time`/`audio_end_time`, and the recording-relative
/// `b` the diarization split computes its boundaries in) is RECORDING-relative
/// seconds. Each word is offset by this row's own `start_seconds` before
/// serializing so `word_timestamps` shares its origin with `audio_start_time`
/// — done here, not by the caller, so it can never drift from that origin.
pub(crate) fn create_transcript_segments_with_words(
    transcripts: &[(String, f64, f64)],
    words: &[Option<Vec<WordStamp>>],
) -> Vec<TranscriptSegment> {
    transcripts
        .iter()
        .enumerate()
        .map(|(idx, (text, start_ms, end_ms))| {
            let start_seconds = start_ms / 1000.0;
            let end_seconds = end_ms / 1000.0;
            let duration = end_seconds - start_seconds;

            TranscriptSegment {
                id: format!("transcript-{}", Uuid::new_v4()),
                text: text.trim().to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                audio_start_time: Some(start_seconds),
                audio_end_time: Some(end_seconds),
                duration: Some(duration),
                // Diarization (specs/0010) runs post-meeting; unset at capture time.
                speaker: None,
                // Unset here: the mixed file carries no channel identity. When the
                // per-channel WAVs exist, retranscription re-derives and fills these
                // tags afterwards (retranscription_channels, 1.10 feedback).
                channel: None,
                word_timestamps: words
                    .get(idx)
                    .and_then(|w| w.as_ref())
                    .map(|w| serialize_word_timestamps(w, start_seconds as f32)),
            }
        })
        .collect()
}

/// Serialize per-word timestamps to the compact `[{"w":..,"s":..,"e":..}, ..]`
/// JSON shape the diarization split reads back, offsetting each word's
/// segment-relative `start`/`end` by `offset_secs` (the row's recording-relative
/// `audio_start_time`) so the stored times are recording-relative too.
fn serialize_word_timestamps(words: &[WordStamp], offset_secs: f32) -> String {
    #[derive(serde::Serialize)]
    struct WordEntry<'a> {
        w: &'a str,
        s: f32,
        e: f32,
    }
    let entries: Vec<WordEntry> = words
        .iter()
        .map(|w| WordEntry {
            w: &w.text,
            s: w.start + offset_secs,
            e: w.end + offset_secs,
        })
        .collect();
    serde_json::to_string(&entries).unwrap_or_default()
}

/// Write transcripts.json to a meeting folder (atomic write with temp file)
pub(crate) fn write_transcripts_json(folder: &Path, segments: &[TranscriptSegment]) -> Result<()> {
    let transcript_path = folder.join("transcripts.json");
    let temp_path = folder.join(".transcripts.json.tmp");

    let json = serde_json::json!({
        "version": "1.0",
        "last_updated": chrono::Utc::now().to_rfc3339(),
        "total_segments": segments.len(),
        "segments": segments.iter().enumerate().map(|(i, s)| {
            serde_json::json!({
                "id": s.id,
                "text": s.text,
                "timestamp": s.timestamp,
                "audio_start_time": s.audio_start_time,
                "audio_end_time": s.audio_end_time,
                "duration": s.duration,
                "sequence_id": i
            })
        }).collect::<Vec<_>>()
    });

    let json_string = serde_json::to_string_pretty(&json)?;
    std::fs::write(&temp_path, &json_string)?;
    std::fs::rename(&temp_path, &transcript_path)?;

    info!(
        "Wrote transcripts.json with {} segments to {}",
        segments.len(),
        transcript_path.display()
    );
    Ok(())
}

/// Split a long speech segment at the lowest-energy (silence) point near the target size.
///
/// Scans for 100ms windows with minimal RMS energy within +/-3 seconds of each target
/// split point. If no clear silence is found, falls back to a 1-second overlap split
/// to avoid cutting words at boundaries.
pub(crate) fn split_segment_at_silence(
    segment: &crate::audio::vad::SpeechSegment,
    max_samples: usize,
) -> Vec<crate::audio::vad::SpeechSegment> {
    const SAMPLE_RATE: usize = 16000;
    // 100ms window for energy measurement (1600 samples at 16kHz)
    const ENERGY_WINDOW: usize = SAMPLE_RATE / 10;
    // Search +/-3 seconds around the target split point
    const SEARCH_RADIUS: usize = SAMPLE_RATE * 3;
    // RMS threshold below which we consider a window "silent"
    const SILENCE_RMS_THRESHOLD: f32 = 0.02;
    // Overlap to use when no silence boundary is found (1 second)
    const FALLBACK_OVERLAP: usize = SAMPLE_RATE;

    let total = segment.samples.len();
    if total <= max_samples {
        return vec![segment.clone()];
    }

    let ms_per_sample =
        (segment.end_timestamp_ms - segment.start_timestamp_ms) / segment.samples.len() as f64;
    let mut result = Vec::new();
    let mut pos = 0usize;

    while pos < total {
        let remaining = total - pos;
        if remaining <= max_samples {
            // Last chunk - take everything remaining
            let chunk_samples = segment.samples[pos..].to_vec();
            let chunk_start_ms = segment.start_timestamp_ms + (pos as f64 * ms_per_sample);
            let chunk_end_ms = segment.end_timestamp_ms;
            result.push(crate::audio::vad::SpeechSegment {
                samples: chunk_samples,
                start_timestamp_ms: chunk_start_ms,
                end_timestamp_ms: chunk_end_ms,
                confidence: segment.confidence,
            });
            break;
        }

        // Target split point
        let target = pos + max_samples;

        // Search window: [target - SEARCH_RADIUS, target + SEARCH_RADIUS]
        let search_start = target.saturating_sub(SEARCH_RADIUS).max(pos + SAMPLE_RATE);
        let search_end = (target + SEARCH_RADIUS).min(total.saturating_sub(ENERGY_WINDOW));

        // Find the lowest-energy 100ms window in the search range
        let mut best_split = target.min(total); // fallback: exact target
        let mut best_rms = f32::MAX;

        if search_start + ENERGY_WINDOW <= search_end {
            let mut idx = search_start;
            while idx + ENERGY_WINDOW <= search_end {
                let window = &segment.samples[idx..idx + ENERGY_WINDOW];
                let rms = (window.iter().map(|s| s * s).sum::<f32>() / ENERGY_WINDOW as f32).sqrt();
                if rms < best_rms {
                    best_rms = rms;
                    best_split = idx + ENERGY_WINDOW / 2; // split at center of quiet window
                }
                // Step by 10ms (160 samples) for efficiency
                idx += SAMPLE_RATE / 100;
            }
        }

        let split_at = best_split;
        if best_rms <= SILENCE_RMS_THRESHOLD {
            debug!(
                "Splitting at silence boundary: sample {} (RMS={:.4})",
                split_at, best_rms
            );
        } else {
            debug!(
                "No silence found near target (best RMS={:.4}), splitting with overlap at sample {}",
                best_rms, split_at
            );
        }

        // Determine the actual end of this chunk (with overlap if no silence)
        let chunk_end = if best_rms > SILENCE_RMS_THRESHOLD {
            (split_at + FALLBACK_OVERLAP).min(total)
        } else {
            split_at
        };

        let chunk_samples = segment.samples[pos..chunk_end].to_vec();
        let chunk_start_ms = segment.start_timestamp_ms + (pos as f64 * ms_per_sample);
        let chunk_end_ms = segment.start_timestamp_ms + (chunk_end as f64 * ms_per_sample);

        result.push(crate::audio::vad::SpeechSegment {
            samples: chunk_samples,
            start_timestamp_ms: chunk_start_ms,
            end_timestamp_ms: chunk_end_ms,
            confidence: segment.confidence,
        });

        // Advance position to where the current chunk actually ends
        // to avoid transcribing the overlap region twice
        pos = chunk_end;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_runs_serialize_in_the_compact_wire_shape() {
        // specs/0055: the live frontend splits a straddling row on these runs, so
        // the field names are a contract (compact like `word_timestamps`).
        let runs = vec![
            ChannelRun {
                start: 12.0,
                end: 13.5,
                tag: ChannelTag::Microphone,
            },
            ChannelRun {
                start: 13.5,
                end: 18.25,
                tag: ChannelTag::Mixed,
            },
        ];
        assert_eq!(
            serde_json::to_string(&runs).expect("serialize channel runs"),
            r#"[{"s":12.0,"e":13.5,"c":"microphone"},{"s":13.5,"e":18.25,"c":"mixed"}]"#
        );
    }

    /// specs/0046 WS2 critical fix: Parakeet's word timestamps are
    /// segment-relative, but `audio_start_time`/`audio_end_time` and the
    /// diarization split's boundaries are recording-relative. A row that
    /// doesn't start at t=0 (every real mid-recording handoff) must have its
    /// stored `word_timestamps` offset onto that same recording-relative
    /// origin, or the split's `word.start >= boundary` search finds no match
    /// and silently keeps the row whole.
    #[test]
    fn word_timestamps_are_offset_to_recording_relative_seconds() {
        let transcripts = vec![("hello world".to_string(), 120_000.0, 122_000.0)];
        let words = vec![Some(vec![
            WordStamp {
                text: "hello".to_string(),
                start: 0.0,
                end: 0.4,
            },
            WordStamp {
                text: "world".to_string(),
                start: 1.5,
                end: 2.0,
            },
        ])];
        let segments = create_transcript_segments_with_words(&transcripts, &words);

        assert_eq!(segments[0].audio_start_time, Some(120.0));
        let parsed: serde_json::Value =
            serde_json::from_str(segments[0].word_timestamps.as_ref().expect("words"))
                .expect("valid json");
        assert!(
            (parsed[0]["s"].as_f64().unwrap() - 120.0).abs() < 1e-4,
            "'hello' should start at recording-relative 120.0, got {}",
            parsed[0]["s"]
        );
        assert!(
            (parsed[1]["s"].as_f64().unwrap() - 121.5).abs() < 1e-4,
            "'world' should start at recording-relative 121.5 (120.0 + 1.5), got {}",
            parsed[1]["s"]
        );
    }

    #[tokio::test]
    async fn test_engine_lifecycle_lock_serializes_acquirers() {
        let guard = acquire_engine_lifecycle_lock().await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (acquired_tx, mut acquired_rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async {
            started_tx.send(()).unwrap();
            let _guard = acquire_engine_lifecycle_lock().await;
            acquired_tx.send(()).unwrap();
        });

        started_rx.await.unwrap();
        assert!(acquired_rx.try_recv().is_err());
        drop(guard);

        acquired_rx.await.unwrap();
        waiter.await.unwrap();
    }
}
