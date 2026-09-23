//! specs/0047 — real-meeting attribution eval (dev/diagnostic, `#[ignore]`).
//!
//! Quantifies, on a REAL two-channel meeting folder (`mic.wav` + `system.wav` +
//! `transcripts.json`), how many transcript segments get labeled "You"
//! (`LOCAL_SPEAKER_KEY`) under the **pre-0047** attribution rule vs the **current
//! (0047)** rule — the "remote speech mislabeled as me" symptom, measured on the
//! user's own audio. Not part of the normal suite; loads a large real recording
//! and runs the slow sherpa diarizer.
//!
//! Run it:
//! ```text
//! export NIXON_EVAL_FOLDER="../zoom-samples/<two-channel recording folder>"
//! cargo test --features metal --lib diarization::real_eval \
//!     -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use crate::audio::decoder::decode_audio_file;
use crate::audio::retranscription::VAD_REDEMPTION_TIME_MS;
use crate::audio::vad::get_speech_chunks;
use crate::diarization::align::{
    align_turns_to_segments, pad_trimmed, AlignableSegment, Channel, LOCAL_SPEAKER_KEY,
};
use crate::diarization::owner_turns::{
    filter_bleed_owner_segments, speech_segments_to_owner_turns,
};
use crate::diarization::segments::channel_from_db;
use crate::diarization::sherpa::SherpaDiarizer;
use crate::diarization::{Diarizer, SpeakerCount, SpeakerTurn};

const SR: u32 = 16_000;

struct Row {
    start: f32,
    end: f32,
    channel: Channel,
    text: String,
}

/// Resolve the two diarization model files across dev/prod identifier dirs.
fn resolve_model_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("NIXON_EVAL_MODEL_DIR") {
        let p = PathBuf::from(d);
        if p.join("segmentation.onnx").exists() {
            return Some(p);
        }
    }
    let data = dirs::data_dir()?;
    for id in [
        "ai.vinyl.app",
        "ai.vinyl.app.debug",
        "com.meetily.ai",
        "Nixon",
    ] {
        let d = data.join(id).join("models").join("diarization");
        if d.join("segmentation.onnx").exists() && d.join("nemo_en_titanet_large.onnx").exists() {
            return Some(d);
        }
    }
    None
}

/// Parse `transcripts.json` into alignable rows (start/end/channel/text).
fn load_rows(folder: &std::path::Path) -> Option<Vec<Row>> {
    let text = std::fs::read_to_string(folder.join("transcripts.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let segs = v["segments"].as_array()?;
    let mut rows = Vec::new();
    for s in segs {
        let (Some(st), Some(en)) = (s["audio_start_time"].as_f64(), s["audio_end_time"].as_f64())
        else {
            continue;
        };
        rows.push(Row {
            start: st as f32,
            end: en as f32,
            channel: channel_from_db(s["channel"].as_str()),
            text: s["text"].as_str().unwrap_or("").to_string(),
        });
    }
    Some(rows)
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Pre-0047 attribution: a System/Mixed segment takes the greatest-overlap turn
/// *including* owner ("local") turns — so a bleed-induced owner turn can win it.
/// (Microphone short-circuits to "You" as it always has; the no-overlap fallback
/// is never "You" since specs/0043 W1.3, so it's collapsed to "unknown" here.)
fn old_align(turns: &[SpeakerTurn], segs: &[AlignableSegment]) -> Vec<String> {
    segs.iter()
        .map(|seg| {
            if seg.channel == Channel::Microphone {
                return LOCAL_SPEAKER_KEY.to_string();
            }
            turns
                .iter()
                .map(|t| (t, overlap(seg.start, seg.end, t.start, t.end)))
                .filter(|(_, ov)| *ov > 0.0)
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(t, _)| t.speaker.clone())
                .unwrap_or_else(|| "unknown".to_string())
        })
        .collect()
}

fn channel_name(c: Channel) -> &'static str {
    match c {
        Channel::Microphone => "microphone",
        Channel::System => "system",
        Channel::Mixed => "mixed",
    }
}

#[test]
#[ignore = "real two-channel meeting eval; run explicitly with --ignored + NIXON_EVAL_FOLDER"]
fn eval_you_attribution_old_vs_new() {
    let Ok(folder) = std::env::var("NIXON_EVAL_FOLDER") else {
        eprintln!("SKIP: set NIXON_EVAL_FOLDER to a meeting folder (mic.wav + system.wav + transcripts.json)");
        return;
    };
    let folder = PathBuf::from(folder);
    let Some(model_dir) = resolve_model_dir() else {
        eprintln!("SKIP: diarization models not present");
        return;
    };
    let Some(rows) = load_rows(&folder) else {
        eprintln!("SKIP: could not read transcripts.json");
        return;
    };

    // Decode both channels (16 kHz mono), same time base.
    // `.wav` or `.opus` (specs/0072).
    use crate::audio::channel_writer::{mic_channel_path, system_channel_path};
    let decode = |p: Option<PathBuf>| {
        let p = p.expect("channel file");
        decode_audio_file(&p)
            .expect("decode channel")
            .to_whisper_format()
    };
    let mic = decode(mic_channel_path(&folder));
    let sys = decode(system_channel_path(&folder));

    // Owner turns from mic-VAD: raw (pre-0047) vs bleed-guarded (0047).
    let raw_segs = get_speech_chunks(&mic, VAD_REDEMPTION_TIME_MS).expect("vad mic");
    let raw_count = raw_segs.len();
    let raw_owner = speech_segments_to_owner_turns(&raw_segs);
    let guarded_segs = filter_bleed_owner_segments(raw_segs, &mic, &sys, SR as usize);
    let guarded_owner = speech_segments_to_owner_turns(&guarded_segs);

    // Remote speaker turns from the system track. PRODUCTION-EQUIVALENT path:
    // with_speaker_count enables specs/0039 same-voice consolidation and
    // diarize_with_embeddings runs it. Speaker-count mode: Auto by default, or
    // AtMost(n) when NIXON_EVAL_ATMOST=n is set (the "seed" mitigation — a
    // calendar/attendee cap). When capping, skip the (slow) raw pass.
    let atmost: Option<u32> = std::env::var("NIXON_EVAL_ATMOST")
        .ok()
        .and_then(|v| v.trim().parse().ok());
    let distinct_raw = if atmost.is_none() {
        SherpaDiarizer::new(
            &model_dir.join("segmentation.onnx"),
            &model_dir.join("nemo_en_titanet_large.onnx"),
        )
        .expect("init raw diarizer")
        .diarize(&sys, SR)
        .expect("raw diarize")
        .iter()
        .map(|t| t.speaker.clone())
        .collect::<std::collections::HashSet<_>>()
        .len()
    } else {
        0
    };

    let speaker_count = match atmost {
        Some(n) => SpeakerCount::AtMost(n),
        None => SpeakerCount::Auto,
    };
    let diarizer = SherpaDiarizer::with_speaker_count(
        &model_dir.join("segmentation.onnx"),
        &model_dir.join("nemo_en_titanet_large.onnx"),
        speaker_count,
    )
    .expect("init diarizer");
    let (remote_turns, _emb) = diarizer
        .diarize_with_embeddings(&sys, SR)
        .expect("diarize system.wav");
    let distinct_remote = remote_turns
        .iter()
        .map(|t| t.speaker.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    // Channel-tagged, pad-trimmed segments (exactly as pipeline::run aligns).
    let alignable: Vec<AlignableSegment> = rows
        .iter()
        .map(|r| {
            let (ts, te) = pad_trimmed(r.start, r.end);
            AlignableSegment::new(ts, te, r.channel)
        })
        .collect();

    // OLD attribution (raw owner turns can win System/Mixed).
    let mut old_turns = remote_turns.clone();
    old_turns.extend(raw_owner.iter().cloned());
    let old_keys = old_align(&old_turns, &alignable);

    // NEW attribution (0047: owner turns filtered from System/Mixed by the real code).
    let mut new_turns = remote_turns.clone();
    new_turns.extend(guarded_owner.iter().cloned());
    let new_keys = align_turns_to_segments(&new_turns, &alignable);

    let old_you = old_keys.iter().filter(|k| *k == LOCAL_SPEAKER_KEY).count();
    let new_you = new_keys.iter().filter(|k| *k == LOCAL_SPEAKER_KEY).count();
    let total = rows.len();

    let mut mic_tagged = 0usize;
    let (mut mixed, mut system) = (0usize, 0usize);
    for r in &rows {
        match r.channel {
            Channel::Microphone => mic_tagged += 1,
            Channel::Mixed => mixed += 1,
            Channel::System => system += 1,
        }
    }

    eprintln!("\n=========== specs/0047 real-meeting attribution eval ===========");
    eprintln!("folder: {}", folder.display());
    eprintln!("segments: {total}  (microphone {mic_tagged} / mixed {mixed} / system {system})");
    eprintln!(
        "owner mic-VAD intervals: {raw_count} raw -> {} kept after bleed guard ({} dropped as bleed)",
        guarded_owner.len(),
        raw_count.saturating_sub(guarded_owner.len())
    );
    match atmost {
        Some(n) => eprintln!(
            "remote speakers on system.wav: {distinct_remote} (AtMost({n}) seed cap — the mitigation)"
        ),
        None => eprintln!(
            "remote speakers on system.wav: {distinct_raw} raw clusters -> {distinct_remote} after \
             specs/0039 consolidation (production Auto path — NO seed)"
        ),
    }
    eprintln!("--------------------------------------------------------------");
    eprintln!("segments labeled \"You\":  OLD (pre-0047) = {old_you}  ->  NEW (0047) = {new_you}",);
    let rescued = old_you.saturating_sub(new_you);
    eprintln!(
        "rescued from a wrong \"You\": {rescued}  ({:.1}% of all segments)",
        100.0 * rescued as f32 / total.max(1) as f32
    );
    eprintln!("--------------------------------------------------------------");

    // Show a sample of segments that flipped OFF "You" — the user can eyeball that
    // these are other people talking.
    eprintln!("sample of segments no longer mislabeled \"You\" (was You -> now):");
    let mut shown = 0;
    for (i, r) in rows.iter().enumerate() {
        if old_keys[i] == LOCAL_SPEAKER_KEY && new_keys[i] != LOCAL_SPEAKER_KEY {
            eprintln!(
                "  [{:>7.1}-{:>7.1}s {:<10}] now={:<8} | {}",
                r.start,
                r.end,
                channel_name(r.channel),
                new_keys[i],
                r.text.chars().take(80).collect::<String>()
            );
            shown += 1;
            if shown >= 25 {
                eprintln!("  … ({} more)", rescued.saturating_sub(25));
                break;
            }
        }
    }
    eprintln!("================================================================\n");
}
