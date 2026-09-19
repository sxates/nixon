//! Owner (microphone) turns for diarization (spec 0046 WS1).
//!
//! Offline diarization runs on the system WAV only, so the owner produces no
//! SpeakerTurn and the 0044 row-splitter can't cut owner↔remote boundaries.
//! Here we derive owner turns from the mic WAV using the same VAD the batch
//! transcription uses, tagged with LOCAL_SPEAKER_KEY. These are injected into the
//! turn set (never into embeddings/clustering — the owner is excluded from
//! voiceprints/gallery/cap elsewhere).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};

use crate::audio::common::ChannelTag;
use crate::audio::pipeline::{classify_window_channel, dominant_channel_for_span};
use crate::audio::vad::SpeechSegment;
use crate::diarization::align::LOCAL_SPEAKER_KEY;
use crate::diarization::SpeakerTurn;

/// Per-window size (ms) for the bleed classification, matching the capture
/// mixer's 600 ms window that [`classify_window_channel`] was tuned against
/// (specs/0029, /0043). Aggregating per-window classes over a segment mirrors the
/// capture path's own per-VAD-segment channel vote.
const BLEED_WINDOW_MS: f64 = 600.0;

/// Owner/system tracks are compared in the 16 kHz mono whisper time base that
/// `DecodedAudio::to_whisper_format` produces (both channels are decoded that way).
const OWNER_VAD_SAMPLE_RATE: usize = 16_000;

/// Drop owner speech intervals that are actually remote audio bleeding from the
/// speakers into the mic (spec 0047). On speakers (no headphones), the remote
/// voice travels speaker → air → mic; because the mic path is EBU R128-normalized
/// to −23 LUFS, that echo is boosted back to speech loudness, so mic-WAV VAD fires
/// on it and — without this guard — mislabels the remote speaker's words as "You"
/// (spec 0046 regression). We reuse the exact per-window bleed guard the capture
/// pipeline already applies to channel tagging (spec 0043 W1.5,
/// [`classify_window_channel`](crate::audio::pipeline)): an interval is kept only
/// when its mic track genuinely dominates the time-aligned system track; bleed
/// classifies as `Mixed`/`System` and is dropped.
///
/// `mic` and `sys` are the full decoded owner/system tracks in the SAME mono
/// 16 kHz time base (both written by `channel_writer` from the same mix windows,
/// so they are sample-aligned from t=0). Pure — no I/O — so it unit-tests without
/// a model or the filesystem. When `sys` is empty (missing/undecodable system
/// track) every interval reads as mic-only and is kept, preserving the pre-0047
/// behavior exactly.
pub(crate) fn filter_bleed_owner_segments(
    segments: Vec<SpeechSegment>,
    mic: &[f32],
    sys: &[f32],
    sample_rate: usize,
) -> Vec<SpeechSegment> {
    // No system track to compare against (missing/undecodable) → every interval
    // reads as mic-only; keep them all, exactly as before 0047.
    if sys.is_empty() || sample_rate == 0 {
        return segments;
    }
    segments
        .into_iter()
        .filter(|seg| owner_interval_is_genuine(seg, mic, sys, sample_rate))
        .collect()
}

/// True when `seg`'s mic track genuinely dominates the time-aligned system track
/// (real owner speech), false when it reads as bleed/overlap (`Mixed`) or
/// system-dominant. Windows the segment span and aggregates the per-window classes
/// with [`dominant_channel_for_span`] — the same vote the capture pipeline uses —
/// so an interval that is mostly the owner with a little bleed still counts as the
/// owner, while an interval that is only speaker echo does not.
fn owner_interval_is_genuine(
    seg: &SpeechSegment,
    mic: &[f32],
    sys: &[f32],
    sample_rate: usize,
) -> bool {
    // A free fn (not a closure) so the returned slice's lifetime ties to `track`.
    fn clamped(track: &[f32], a: usize, b: usize) -> &[f32] {
        let a = a.min(track.len());
        let b = b.min(track.len()).max(a);
        &track[a..b]
    }

    let win = ((sample_rate as f64) * BLEED_WINDOW_MS / 1000.0) as usize;
    if win == 0 {
        return true;
    }
    let to_idx = |ms: f64| ((ms / 1000.0) * sample_rate as f64).max(0.0) as usize;
    let to_ms = |idx: usize| (idx as f64) / (sample_rate as f64) * 1000.0;

    let start = to_idx(seg.start_timestamp_ms);
    let end = to_idx(seg.end_timestamp_ms).max(start);

    let mut windows: VecDeque<(f64, f64, ChannelTag)> = VecDeque::new();
    let mut w = start;
    while w < end {
        let we = (w + win).min(end);
        if let Some(tag) = classify_window_channel(clamped(mic, w, we), clamped(sys, w, we)) {
            windows.push_back((to_ms(w), to_ms(we), tag));
        }
        w = we;
    }

    match dominant_channel_for_span(&windows, to_ms(start), to_ms(end)) {
        // The owner clearly leads across the interval → keep it as an owner turn.
        Some(ChannelTag::Microphone) => true,
        // `Mixed` (bleed guard tripped, or genuinely overlapped speech) and
        // `System` (remote-led) are both too ambiguous to hard-label "You" — drop
        // the owner turn and let alignment resolve those segments via the diarized
        // remote turns / Unknown bucket instead (align.rs, specs/0043 W1.3).
        Some(_) => false,
        // No classifiable audio at all — shouldn't happen for a VAD-detected owner
        // segment (the mic must have had energy). Keep it rather than silently
        // deleting the owner's speech on a degenerate window.
        None => true,
    }
}

/// Pure: VAD speech intervals (ms) → owner turns (recording-relative seconds).
pub fn speech_segments_to_owner_turns(segments: &[SpeechSegment]) -> Vec<SpeakerTurn> {
    segments
        .iter()
        .map(|s| SpeakerTurn {
            start: (s.start_timestamp_ms / 1000.0) as f32,
            end: (s.end_timestamp_ms / 1000.0) as f32,
            speaker: LOCAL_SPEAKER_KEY.to_string(),
        })
        .collect()
}

/// Resolve the mic WAV for this meeting folder, decode it, VAD it, and return
/// owner turns. Returns an EMPTY vec on any problem (missing mic WAV, decode
/// error) so diarization behaves exactly as before when the owner track is
/// unavailable — never fatal.
pub async fn owner_turns_for_meeting<R: Runtime>(
    _app: &AppHandle<R>,
    meeting_folder: &Path,
) -> Vec<SpeakerTurn> {
    let mic_wav = crate::audio::channel_writer::mic_channel_wav(meeting_folder);
    if !mic_wav.exists() {
        log::info!("owner turns: no mic WAV at {mic_wav:?} — skipping owner track");
        return Vec::new();
    }
    // Decode → mono 16k, then VAD with the batch redemption window.
    let decoded = match crate::audio::decoder::decode_audio_file(&mic_wav) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("owner turns: mic WAV decode failed ({e:#}) — skipping owner track");
            return Vec::new();
        }
    };
    let mic_samples = decoded.to_whisper_format();

    // Decode the system track too, so the bleed guard (spec 0047) can reject
    // intervals that are really remote audio leaking from the speakers into the
    // mic. Best-effort: on a missing/undecodable system track we pass an empty
    // slice, which `filter_bleed_owner_segments` treats as "nothing to compare"
    // and keeps every interval — i.e. exactly the pre-0047 behavior.
    let sys_wav = crate::audio::channel_writer::system_channel_wav(meeting_folder);
    let sys_samples = if sys_wav.exists() {
        match crate::audio::decoder::decode_audio_file(&sys_wav) {
            Ok(d) => d.to_whisper_format(),
            Err(e) => {
                log::warn!(
                    "owner turns: system WAV decode failed ({e:#}) — bleed guard off for this meeting"
                );
                Vec::new()
            }
        }
    } else {
        log::info!("owner turns: no system WAV at {sys_wav:?} — bleed guard off for this meeting");
        Vec::new()
    };

    // Run the blocking VAD off the async runtime, then drop speaker-bleed
    // intervals before they can become "You" turns (spec 0047).
    let segments = tokio::task::spawn_blocking(move || {
        crate::audio::vad::get_speech_chunks(
            &mic_samples,
            crate::audio::retranscription::VAD_REDEMPTION_TIME_MS,
        )
        .map(|segs| {
            filter_bleed_owner_segments(segs, &mic_samples, &sys_samples, OWNER_VAD_SAMPLE_RATE)
        })
    })
    .await;
    match segments {
        Ok(Ok(segs)) => {
            let turns = speech_segments_to_owner_turns(&segs);
            log::info!(
                "owner turns: {} owner speech interval(s) from mic WAV",
                turns.len()
            );
            turns
        }
        Ok(Err(e)) => {
            log::warn!("owner turns: VAD failed ({e:#}) — skipping owner track");
            Vec::new()
        }
        Err(e) => {
            log::warn!("owner turns: VAD task panicked ({e}) — skipping owner track");
            Vec::new()
        }
    }
}

/// Resolve this meeting's recording folder from its persisted `folder_path`.
///
/// By the time offline diarization (`pipeline::run`) reaches this call, it has
/// already resolved + backfilled `folder_path` via `resolve_system_wav` earlier
/// in the same pass, so a plain lookup suffices — no need to repeat its
/// recordings-root scan. Returns `None` on any DB miss (never fatal).
async fn resolve_meeting_folder(pool: &sqlx::SqlitePool, meeting_id: &str) -> Option<PathBuf> {
    let meta = crate::database::repositories::meeting::MeetingsRepository::get_meeting_metadata(
        pool, meeting_id,
    )
    .await
    .ok()??;
    meta.folder_path.map(PathBuf::from)
}

/// Pure: merge `owner` turns into `turns`, re-sorted by start. No-op when `owner`
/// is empty (`turns` is left untouched, unsorted).
fn merge_owner_turns(turns: &mut Vec<SpeakerTurn>, owner: Vec<SpeakerTurn>) {
    if owner.is_empty() {
        return;
    }
    turns.extend(owner);
    turns.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Resolve and merge this meeting's owner turns into `turns` (spec 0046 W1.2).
/// Only ever extends `turns` — never touches embeddings, so the owner stays
/// excluded from voiceprints/gallery/cap. A no-op when the folder or mic WAV is
/// unavailable (see [`owner_turns_for_meeting`]).
pub async fn inject_owner_turns<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    turns: &mut Vec<SpeakerTurn>,
) {
    let Some(folder) = resolve_meeting_folder(pool, meeting_id).await else {
        log::info!("owner turns: no folder_path for meeting {meeting_id} — skipping owner track");
        return;
    };
    let owner_turns = owner_turns_for_meeting(app, &folder).await;
    let injected = owner_turns.len();
    merge_owner_turns(turns, owner_turns);
    if injected > 0 {
        log::info!(
            "Diarization: injected {injected} owner turn(s); {} total turns for {meeting_id}",
            turns.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_bleed_owner_segments, merge_owner_turns, speech_segments_to_owner_turns};
    use crate::audio::vad::SpeechSegment;
    use crate::diarization::align::LOCAL_SPEAKER_KEY;
    use crate::diarization::SpeakerTurn;

    fn seg(start_ms: f64, end_ms: f64) -> SpeechSegment {
        SpeechSegment {
            samples: vec![],
            start_timestamp_ms: start_ms,
            end_timestamp_ms: end_ms,
            confidence: 1.0,
        }
    }

    #[test]
    fn converts_ms_intervals_to_local_turns_in_seconds() {
        let turns = speech_segments_to_owner_turns(&[seg(0.0, 1500.0), seg(3400.0, 6300.0)]);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].speaker, LOCAL_SPEAKER_KEY);
        assert!((turns[0].start - 0.0).abs() < 1e-6 && (turns[0].end - 1.5).abs() < 1e-6);
        assert!((turns[1].start - 3.4).abs() < 1e-6 && (turns[1].end - 6.3).abs() < 1e-6);
    }

    #[test]
    fn empty_segments_yield_no_turns() {
        assert!(speech_segments_to_owner_turns(&[]).is_empty());
    }

    // --- spec 0047: speaker-bleed guard on owner turns ---

    /// Fill `track[start_ms..end_ms)` (16 kHz) with a constant giving RMS = `level`.
    fn fill(track: &mut [f32], start_ms: f64, end_ms: f64, level: f32) {
        let sr = 16_000.0;
        let s = ((start_ms / 1000.0) * sr) as usize;
        let e = (((end_ms / 1000.0) * sr) as usize).min(track.len());
        for x in &mut track[s..e] {
            *x = level;
        }
    }

    #[test]
    fn bleed_only_interval_is_dropped_while_genuine_owner_is_kept() {
        // Two back-to-back 1 s intervals over a 2 s mic/system pair.
        //   [0,1s): genuine owner  — mic at speech loudness, system silent.
        //   [1,2s): pure bleed     — mic (normalized echo) is louder, but the
        //           system track is SIMULTANEOUSLY at speech loudness, which is
        //           the tell that the mic energy is remote playback, not the
        //           owner. It must be rejected (spec 0043 W1.5 reused).
        let sr = 16_000usize;
        let mut mic = vec![0.0f32; sr * 2];
        let mut sys = vec![0.0f32; sr * 2];
        fill(&mut mic, 0.0, 1000.0, 0.10); // genuine owner speech
        fill(&mut mic, 1000.0, 2000.0, 0.18); // bleed: mic-dominant echo
        fill(&mut sys, 1000.0, 2000.0, 0.06); // system playing at speech loudness

        let kept = filter_bleed_owner_segments(
            vec![seg(0.0, 1000.0), seg(1000.0, 2000.0)],
            &mic,
            &sys,
            sr,
        );

        assert_eq!(kept.len(), 1, "the bleed interval must be dropped");
        assert!(
            (kept[0].start_timestamp_ms - 0.0).abs() < 1e-6,
            "the surviving interval must be the genuine owner one at t=0"
        );
    }

    #[test]
    fn missing_system_track_keeps_every_owner_interval() {
        // No system track (empty `sys`) → nothing to compare against → the guard
        // must not silently delete the owner's speech (preserves pre-0047 behavior).
        let sr = 16_000usize;
        let mut mic = vec![0.0f32; sr];
        fill(&mut mic, 0.0, 1000.0, 0.10);
        let kept = filter_bleed_owner_segments(vec![seg(0.0, 1000.0)], &mic, &[], sr);
        assert_eq!(kept.len(), 1);
    }

    fn turn(start: f32, end: f32, speaker: &str) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker: speaker.to_string(),
        }
    }

    #[test]
    fn merge_owner_turns_extends_and_sorts_by_start() {
        let mut turns = vec![turn(0.0, 1.0, "spk_0"), turn(5.0, 6.0, "spk_1")];
        let owner = vec![turn(2.0, 3.0, LOCAL_SPEAKER_KEY)];
        merge_owner_turns(&mut turns, owner);
        assert_eq!(turns.len(), 3);
        assert_eq!(
            turns.iter().map(|t| t.speaker.as_str()).collect::<Vec<_>>(),
            vec!["spk_0", LOCAL_SPEAKER_KEY, "spk_1"]
        );
    }

    #[test]
    fn merge_owner_turns_is_noop_on_empty_owner() {
        let mut turns = vec![turn(0.0, 1.0, "spk_0")];
        merge_owner_turns(&mut turns, vec![]);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].speaker, "spk_0");
    }
}
