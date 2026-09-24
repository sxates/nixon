//! Capture-channel tagging for batch retranscription (1.10 feedback).
//!
//! A deferred (record-only) meeting is transcribed later from the MIXED audio
//! file, which destroys channel identity — every row used to be persisted with
//! `channel = NULL`, so the offline diarization pass could never attribute the
//! local user's segments to "You" (align.rs maps NULL to `Mixed`), and the user
//! had to reassign an "Unknown speaker" to themselves by hand.
//!
//! But the recording pipeline ALSO writes clean per-channel 16 kHz WAVs
//! (`mic.wav` / `system.wav`, specs/0010 P1-B1) on the same timeline as the
//! mixed file. This module re-derives the per-segment channel tag offline the
//! same way the live pipeline does it (specs/0029 WS3.4): windowed RMS
//! dominance of the two pre-mix tracks, aggregated over each VAD segment's
//! span. Classification reuses the live classifier verbatim —
//! [`classify_window_channel`] only looks at window RMS, so feeding it a
//! single-sample slice containing a window's precomputed RMS yields exactly
//! the live tag for that window.
//!
//! Everything here is best-effort: any missing/undecodable/duration-mismatched
//! track yields `None` tags (persisted as NULL — the pre-fix behavior), never
//! an error that could fail the retranscription itself.

use crate::audio::channel_attribution::CHANNEL_ACTIVE_RMS;
use crate::audio::common::ChannelTag;
use crate::audio::pipeline::{classify_window_channel, dominant_channel_for_span};
use log::{info, warn};
use std::collections::VecDeque;
use std::path::Path;

/// Classification window: 600 ms (mirrors the live mixer's window length).
const WINDOW_MS: f64 = 600.0;

/// Max tolerated difference between the mixed file's duration and a channel
/// track's duration before the tracks are considered a different timeline and
/// tagging is skipped. Covers writer flush/rounding slack; a resumed session —
/// whose channel WAVs cover only the final segment while the mixed file is the
/// full concat — lands far outside it.
const MAX_DURATION_DRIFT_SECONDS: f64 = 5.0;

/// Per-window RMS profiles of the two capture-channel WAVs, on the mixed
/// file's timeline. Memory-light: one f32 per 600 ms window per track.
pub struct ChannelRmsProfile {
    mic: Vec<f32>,
    system: Vec<f32>,
    /// `false` when the meeting has no system channel file at all (specs/0078: a
    /// mic-only folder reads as a room recording, not as a silent call).
    system_present: bool,
}

impl ChannelRmsProfile {
    /// Build a profile from already-decoded 16 kHz mono tracks (test seam; the
    /// file path is [`ChannelRmsProfile::load`]).
    pub fn from_tracks(mic: &[f32], system: &[f32], sample_rate: u32) -> Self {
        let window = ((sample_rate as f64) * WINDOW_MS / 1000.0).round().max(1.0) as usize;
        Self {
            mic: windowed_rms(mic, window),
            system: windowed_rms(system, window),
            system_present: true,
        }
    }

    /// A mic-only profile, for a meeting with no system channel file (specs/0078).
    pub fn from_mic_track(mic: &[f32], sample_rate: u32) -> Self {
        let mut profile = Self::from_tracks(mic, &[], sample_rate);
        profile.system_present = false;
        profile
    }

    /// Load the channel files for room detection (specs/0078). Unlike [`Self::load`]
    /// there is no mixed-file duration to check against: the two tracks are only
    /// compared with each other. `None` when the mic channel is missing, or when either
    /// present track can't be decoded (detection then falls back to call mode). A
    /// missing system channel gives a mic-only profile.
    pub fn load_for_detection(folder: &Path) -> Option<Self> {
        let mic_path = crate::audio::channel_writer::mic_channel_path(folder)?;
        let mic = decode_track_rms(&mic_path, None)?;
        let system_path = crate::audio::channel_writer::system_channel_path(folder);
        let (system, system_present) = match system_path {
            Some(p) => (decode_track_rms(&p, None)?, true),
            None => (Vec::new(), false),
        };
        Some(Self {
            mic,
            system,
            system_present,
        })
    }

    /// How much each track is active, measured with the capture classifier's own 600 ms
    /// windows and [`CHANNEL_ACTIVE_RMS`] bar, so "active" means what it means for
    /// `transcripts.channel` (specs/0078 room detection).
    pub fn activity(&self) -> crate::diarization::room::ChannelActivity {
        let window_secs = (WINDOW_MS / 1000.0) as f32;
        let is_active = |rms: &f32| *rms >= CHANNEL_ACTIVE_RMS;
        let mut longest = 0usize;
        let mut run = 0usize;
        for rms in &self.system {
            run = if is_active(rms) { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        crate::diarization::room::ChannelActivity {
            duration_secs: self.mic.len().max(self.system.len()) as f32 * window_secs,
            system_active_secs: self.system.iter().filter(|r| is_active(r)).count() as f32
                * window_secs,
            system_longest_run_secs: longest as f32 * window_secs,
            mic_active_secs: self.mic.iter().filter(|r| is_active(r)).count() as f32 * window_secs,
            system_present: self.system_present,
        }
    }

    /// Load `mic.wav` + `system.wav` from a meeting folder and reduce them to
    /// per-window RMS. `None` (skip tagging) when either file is missing or
    /// unreadable, or when either track's duration drifts more than
    /// [`MAX_DURATION_DRIFT_SECONDS`] from `expected_duration_seconds` (the
    /// decoded MIXED file's duration) — a mismatched timeline would attribute
    /// the wrong speaker, which is worse than no tag.
    pub fn load(folder: &Path, expected_duration_seconds: f64) -> Option<Self> {
        // `.wav`, or `.opus` once kept audio is compressed (specs/0072).
        let mic_path = crate::audio::channel_writer::mic_channel_path(folder);
        let system_path = crate::audio::channel_writer::system_channel_path(folder);
        let (Some(mic_path), Some(system_path)) = (mic_path, system_path) else {
            info!(
                "No per-channel audio in {} — retranscription proceeds untagged",
                folder.display()
            );
            return None;
        };
        // Sequential decode-and-reduce keeps the peak footprint to one track's
        // samples; only the tiny per-window RMS vectors are retained.
        let mic = decode_track_rms(&mic_path, Some(expected_duration_seconds))?;
        let system = decode_track_rms(&system_path, Some(expected_duration_seconds))?;
        Some(Self {
            mic,
            system,
            system_present: true,
        })
    }

    /// Tag each `(start_ms, end_ms)` span with its dominant capture channel,
    /// exactly like the live pipeline: classify every 600 ms window by RMS
    /// dominance, then take the overlap-time-weighted majority across the
    /// span's windows. `None` = no classified (non-silent) window overlaps.
    pub fn classify_spans(&self, spans: &[(f64, f64)]) -> Vec<Option<ChannelTag>> {
        // classify_window_channel computes the RMS of the slices it is given; a
        // single-sample slice holding a window's precomputed RMS round-trips
        // that value exactly, so this IS the live per-window classification.
        let windows: VecDeque<(f64, f64, ChannelTag)> = self
            .mic
            .iter()
            .zip(self.system.iter().chain(std::iter::repeat(&0.0f32)))
            .enumerate()
            .filter_map(|(i, (&mic_rms, &sys_rms))| {
                classify_window_channel(&[mic_rms], &[sys_rms])
                    .map(|tag| (i as f64 * WINDOW_MS, (i + 1) as f64 * WINDOW_MS, tag))
            })
            .collect();
        spans
            .iter()
            .map(|&(start_ms, end_ms)| dominant_channel_for_span(&windows, start_ms, end_ms))
            .collect()
    }
}

/// Reduce a track to per-600 ms-window RMS values.
fn windowed_rms(samples: &[f32], window: usize) -> Vec<f32> {
    samples
        .chunks(window)
        .map(|chunk| (chunk.iter().map(|&s| s * s).sum::<f32>() / chunk.len() as f32).sqrt())
        .collect()
}

/// Decode one channel WAV to its per-window RMS profile, enforcing the
/// duration guard when an expected duration is given. Best-effort: any failure
/// logs and returns `None`.
fn decode_track_rms(path: &Path, expected_duration_seconds: Option<f64>) -> Option<Vec<f32>> {
    let decoded = match crate::audio::decoder::decode_audio_file(path) {
        Ok(decoded) => decoded,
        Err(e) => {
            warn!(
                "Failed to decode {} for channel tagging (proceeding untagged): {}",
                path.display(),
                e
            );
            return None;
        }
    };
    let expected_duration_seconds = expected_duration_seconds.unwrap_or(decoded.duration_seconds);
    let drift = (decoded.duration_seconds - expected_duration_seconds).abs();
    if drift > MAX_DURATION_DRIFT_SECONDS {
        warn!(
            "{} duration {:.1}s drifts {:.1}s from the mixed audio's {:.1}s (resumed \
             session?) — skipping channel tagging rather than mis-attributing speakers",
            path.display(),
            decoded.duration_seconds,
            drift,
            expected_duration_seconds
        );
        return None;
    }
    let samples = decoded.to_whisper_format();
    Some(windowed_rms(
        &samples,
        ((16_000.0 * WINDOW_MS / 1000.0) as usize).max(1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// A track that is `active` (speech-loud, RMS 0.1) over `[start_s, end_s)`
    /// and silent elsewhere, `total_s` long.
    fn track(total_s: f64, active: &[(f64, f64)]) -> Vec<f32> {
        let mut samples = vec![0.0f32; (total_s * RATE as f64) as usize];
        for &(start_s, end_s) in active {
            let a = (start_s * RATE as f64) as usize;
            let b = ((end_s * RATE as f64) as usize).min(samples.len());
            for s in &mut samples[a..b] {
                *s = 0.1;
            }
        }
        samples
    }

    #[test]
    fn tags_mic_only_speech_as_microphone() {
        let mic = track(3.0, &[(0.0, 3.0)]);
        let sys = track(3.0, &[]);
        let profile = ChannelRmsProfile::from_tracks(&mic, &sys, RATE);
        assert_eq!(
            profile.classify_spans(&[(200.0, 2_800.0)]),
            vec![Some(ChannelTag::Microphone)]
        );
    }

    #[test]
    fn tags_system_only_speech_as_system() {
        let mic = track(3.0, &[]);
        let sys = track(3.0, &[(0.0, 3.0)]);
        let profile = ChannelRmsProfile::from_tracks(&mic, &sys, RATE);
        assert_eq!(
            profile.classify_spans(&[(200.0, 2_800.0)]),
            vec![Some(ChannelTag::System)]
        );
    }

    #[test]
    fn tags_overlapped_speech_as_mixed() {
        // Both tracks active at comparable loudness across the span.
        let mic = track(3.0, &[(0.0, 3.0)]);
        let sys = track(3.0, &[(0.0, 3.0)]);
        let profile = ChannelRmsProfile::from_tracks(&mic, &sys, RATE);
        assert_eq!(
            profile.classify_spans(&[(200.0, 2_800.0)]),
            vec![Some(ChannelTag::Mixed)]
        );
    }

    #[test]
    fn leaves_silence_untagged() {
        let mic = track(3.0, &[]);
        let sys = track(3.0, &[]);
        let profile = ChannelRmsProfile::from_tracks(&mic, &sys, RATE);
        assert_eq!(profile.classify_spans(&[(200.0, 2_800.0)]), vec![None]);
    }

    #[test]
    fn tags_each_span_independently() {
        // Mic talks 0–2 s, system talks 3–5 s (a turn-taking conversation).
        let mic = track(6.0, &[(0.0, 2.0)]);
        let sys = track(6.0, &[(3.0, 5.0)]);
        let profile = ChannelRmsProfile::from_tracks(&mic, &sys, RATE);
        assert_eq!(
            profile.classify_spans(&[(100.0, 1_900.0), (3_100.0, 4_900.0)]),
            vec![Some(ChannelTag::Microphone), Some(ChannelTag::System)]
        );
    }

    #[test]
    fn load_returns_none_when_channel_wavs_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ChannelRmsProfile::load(dir.path(), 10.0).is_none());
    }

    /// Write a 16 kHz mono s16 WAV through the same writer the pipeline uses
    /// (input windows are 48 kHz, downsampled 3:1 internally).
    fn write_channel_wav(path: std::path::PathBuf, seconds: f64, amplitude: f32) {
        let mut writer = crate::audio::channel_writer::ChannelWavWriter::new(path).unwrap();
        let window_48k = vec![amplitude; (48_000.0 * 0.6) as usize];
        let windows = (seconds / 0.6).round() as usize;
        for _ in 0..windows {
            writer.write_window(&window_48k).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn load_reads_real_channel_wavs_and_classifies() {
        let dir = tempfile::tempdir().unwrap();
        write_channel_wav(
            crate::audio::channel_writer::mic_channel_wav(dir.path()),
            6.0,
            0.1,
        );
        write_channel_wav(
            crate::audio::channel_writer::system_channel_wav(dir.path()),
            6.0,
            0.0,
        );
        let profile = ChannelRmsProfile::load(dir.path(), 6.0).expect("profile should load");
        assert_eq!(
            profile.classify_spans(&[(500.0, 5_500.0)]),
            vec![Some(ChannelTag::Microphone)]
        );
    }

    /// specs/0078: activity counts 600 ms windows at or above the classifier's bar, and
    /// the longest run of consecutive active system windows.
    #[test]
    fn activity_measures_active_time_and_the_longest_system_run() {
        // 12 s: system active 0–1.2 s and 3.0–6.0 s; mic active 0–9 s.
        let mic = track(12.0, &[(0.0, 9.0)]);
        let sys = track(12.0, &[(0.0, 1.2), (3.0, 6.0)]);
        let a = ChannelRmsProfile::from_tracks(&mic, &sys, RATE).activity();
        assert!((a.duration_secs - 12.0).abs() < 1e-3, "{a:?}");
        assert!((a.system_active_secs - 4.2).abs() < 1e-3, "{a:?}");
        assert!((a.system_longest_run_secs - 3.0).abs() < 1e-3, "{a:?}");
        assert!((a.mic_active_secs - 9.0).abs() < 1e-3, "{a:?}");
        assert!(a.system_present);

        let solo = ChannelRmsProfile::from_mic_track(&mic, RATE).activity();
        assert!(!solo.system_present);
        assert_eq!(solo.system_active_secs, 0.0);
        assert!((solo.mic_active_secs - 9.0).abs() < 1e-3);
    }

    #[test]
    fn load_for_detection_reads_a_mic_only_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            ChannelRmsProfile::load_for_detection(dir.path()).is_none(),
            "no mic channel means nothing to detect from"
        );
        write_channel_wav(
            crate::audio::channel_writer::mic_channel_wav(dir.path()),
            6.0,
            0.1,
        );
        let a = ChannelRmsProfile::load_for_detection(dir.path())
            .expect("mic-only folder loads")
            .activity();
        assert!(!a.system_present);
        assert!(a.mic_active_secs > 5.0, "{a:?}");
    }

    #[test]
    fn load_rejects_a_duration_mismatch() {
        // A resumed session: mixed audio is the full concat (60 s) but the
        // channel WAVs were truncated to the final segment (6 s).
        let dir = tempfile::tempdir().unwrap();
        write_channel_wav(
            crate::audio::channel_writer::mic_channel_wav(dir.path()),
            6.0,
            0.1,
        );
        write_channel_wav(
            crate::audio::channel_writer::system_channel_wav(dir.path()),
            6.0,
            0.0,
        );
        assert!(ChannelRmsProfile::load(dir.path(), 60.0).is_none());
    }
}
