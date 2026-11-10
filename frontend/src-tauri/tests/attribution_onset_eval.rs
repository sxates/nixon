//! Speaker-onset attribution eval (dev/diagnostic, `#[ignore]`).
//!
//! Owner report (2026-11-09): "the first sentence of a speaker gets attributed to the
//! previous speaker". This replays the PRODUCTION row-split + alignment code
//! (`split::plan_split` → `align::pad_trimmed` → `align::align_turns_to_segments`) on
//! the Zoom ground-truth samples and counts words attributed to the wrong speaker,
//! with the subset that sits at the START of a reference turn ("onset" words) called
//! out separately.
//!
//! Inputs (local only, never in the repo): `zoom-samples/<sample>/` with one `.vtt`
//! and the `.eval-cache/` written by `diarization_tuning.rs` (16 kHz mono WAV + raw
//! diarization turns). Rows come from the app's own Silero VAD at the live redemption
//! (400 ms) or batch (2000 ms) window. Words are the VTT cue words spread linearly over
//! each cue, so STT timing error is NOT modelled — this isolates the cut/label logic.
//!
//! Turns are relabeled to their best-overlap reference speaker ("oracle clustering") so
//! clustering confusion doesn't drown the boundary signal; set `ONSET_RAW_LABELS=1` to
//! keep the raw cluster labels instead.
//!
//! ```text
//! cargo test --features metal --test attribution_onset_eval -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use app_lib::audio::decoder::decode_audio_file;
use app_lib::audio::vad::get_speech_chunks;
use app_lib::diarization::align::{
    align_turns_to_segments, pad_trimmed, AlignMode, AlignableSegment, Channel,
};
use app_lib::diarization::split::{plan_split, WordStamp};
use app_lib::diarization::SpeakerTurn;

/// Words at the start of each reference turn that count as its "first sentence".
const ONSET_WORDS: usize = 8;

struct Cue {
    start: f32,
    end: f32,
    speaker: String,
    text: String,
}

#[derive(Clone)]
struct Word {
    text: String,
    start: f32,
    end: f32,
    speaker: String,
    /// Index of this word within its reference turn (consecutive same-speaker cues).
    turn_pos: usize,
}

fn eval_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("NIXON_DIARIZATION_EVAL_DIR") {
        return Some(PathBuf::from(d));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        root.join("../zoom-samples"),
        root.join("../../zoom-samples"),
    ]
    .into_iter()
    .filter_map(|p| p.canonicalize().ok())
    .find(|p| p.is_dir())
}

fn ts(s: &str) -> Option<f32> {
    let mut it = s.trim().split(':');
    let h: f32 = it.next()?.parse().ok()?;
    let m: f32 = it.next()?.parse().ok()?;
    let sec: f32 = it.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn parse_vtt(p: &Path) -> Vec<Cue> {
    let text = std::fs::read_to_string(p).expect("read vtt");
    let lines: Vec<&str> = text.lines().collect();
    let mut cues = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((a, b)) = lines[i].split_once("-->") {
            let body = lines.get(i + 1).copied().unwrap_or("");
            if let (Some(s), Some(e), Some((spk, t))) = (
                ts(a),
                ts(b.split_whitespace().next().unwrap_or("")),
                body.split_once(':'),
            ) {
                if spk.len() <= 60 && e > s {
                    cues.push(Cue {
                        start: s,
                        end: e,
                        speaker: spk.trim().to_string(),
                        text: t.trim().to_string(),
                    });
                }
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    cues
}

fn cue_words(cues: &[Cue]) -> Vec<Word> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut prev_spk: Option<&str> = None;
    for c in cues {
        if prev_spk != Some(c.speaker.as_str()) {
            pos = 0;
        }
        prev_spk = Some(c.speaker.as_str());
        let toks: Vec<&str> = c.text.split_whitespace().collect();
        let n = toks.len().max(1) as f32;
        let step = (c.end - c.start) / n;
        for (k, w) in toks.iter().enumerate() {
            out.push(Word {
                text: (*w).to_string(),
                start: c.start + step * k as f32,
                end: c.start + step * (k as f32 + 1.0),
                speaker: c.speaker.clone(),
                turn_pos: pos,
            });
            pos += 1;
        }
    }
    out
}

#[derive(serde::Deserialize)]
struct RawDiar {
    turns: Vec<(f32, f32, String)>,
}

fn load_turns(cache: &Path) -> Option<Vec<SpeakerTurn>> {
    let p = cache.join("raw-diar.nemo_en_titanet_large.th0.8.v1.json");
    let raw: RawDiar = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
    Some(
        raw.turns
            .into_iter()
            .map(|(start, end, speaker)| SpeakerTurn {
                start,
                end,
                speaker,
            })
            .collect(),
    )
}

/// Relabel each cluster to the reference speaker it overlaps most, then merge
/// touching same-label turns (what perfect clustering would have produced).
fn oracle_relabel(turns: &[SpeakerTurn], cues: &[Cue]) -> Vec<SpeakerTurn> {
    let mut ov: HashMap<(String, String), f32> = HashMap::new();
    for t in turns {
        for c in cues {
            let o = t.end.min(c.end) - t.start.max(c.start);
            if o > 0.0 {
                *ov.entry((t.speaker.clone(), c.speaker.clone()))
                    .or_default() += o;
            }
        }
    }
    let mut best: HashMap<String, (String, f32)> = HashMap::new();
    for ((k, r), o) in ov {
        let e = best.entry(k).or_insert((r.clone(), 0.0));
        if o > e.1 {
            *e = (r, o);
        }
    }
    let mut out: Vec<SpeakerTurn> = Vec::new();
    for t in turns {
        let spk = best
            .get(&t.speaker)
            .map(|(r, _)| r.clone())
            .unwrap_or_else(|| t.speaker.clone());
        match out.last_mut() {
            Some(last) if last.speaker == spk && t.start <= last.end + 0.01 => {
                last.end = last.end.max(t.end)
            }
            _ => out.push(SpeakerTurn {
                start: t.start,
                end: t.end,
                speaker: spk,
            }),
        }
    }
    out
}

fn vad_rows(cache: &Path, wav: &Path, redemption: u32) -> Vec<(f32, f32)> {
    let p = cache.join(format!("onset-vad.r{redemption}.v1.json"));
    if let Ok(b) = std::fs::read(&p) {
        if let Ok(v) = serde_json::from_slice::<Vec<(f32, f32)>>(&b) {
            return v;
        }
    }
    let samples = decode_audio_file(wav).expect("decode").to_whisper_format();
    let rows: Vec<(f32, f32)> = get_speech_chunks(&samples, redemption)
        .expect("vad")
        .iter()
        .map(|s| {
            (
                (s.start_timestamp_ms / 1000.0) as f32,
                (s.end_timestamp_ms / 1000.0) as f32,
            )
        })
        .collect();
    std::fs::write(&p, serde_json::to_vec(&rows).unwrap()).ok();
    rows
}

#[derive(Default)]
struct Score {
    words: usize,
    wrong: usize,
    onset: usize,
    onset_wrong: usize,
    /// Onset words given to the speaker of the PREVIOUS reference turn.
    onset_to_prev: usize,
    turns: usize,
    /// Turns whose first word was mislabeled.
    turns_first_wrong: usize,
}

/// Production path: words → rows, plan_split each row, align every part.
/// Returns the predicted speaker per word index.
fn production_labels(
    rows: &[(f32, f32)],
    words: &[Word],
    turns: &[SpeakerTurn],
) -> Vec<Option<String>> {
    // Assign each word to the row containing its midpoint.
    let mut row_words: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
    let mut r = 0usize;
    for (i, w) in words.iter().enumerate() {
        let mid = (w.start + w.end) / 2.0;
        while r < rows.len() && rows[r].1 < mid {
            r += 1;
        }
        if r < rows.len() && rows[r].0 <= mid {
            row_words[r].push(i);
        }
    }
    let mut pred = vec![None; words.len()];
    let per_word = std::env::var("ONSET_MODE").as_deref() == Ok("word");
    for (ri, &(rs, re)) in rows.iter().enumerate() {
        let idx = &row_words[ri];
        if idx.is_empty() {
            continue;
        }
        let text: String = idx
            .iter()
            .map(|&i| words[i].text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let stamps: Vec<WordStamp> = idx
            .iter()
            .map(|&i| {
                serde_json::from_value(serde_json::json!({
                    "w": words[i].text, "s": words[i].start, "e": words[i].end
                }))
                .unwrap()
            })
            .collect();
        // (part_start, part_end, word count)
        if per_word {
            let segs: Vec<AlignableSegment> = idx
                .iter()
                .map(|&i| AlignableSegment::new(words[i].start, words[i].end, Channel::System))
                .collect();
            for (&wi, k) in idx
                .iter()
                .zip(align_turns_to_segments(turns, &segs, AlignMode::Call))
            {
                pred[wi] = Some(k);
            }
            continue;
        }
        let parts: Vec<(f32, f32, usize)> = match plan_split(rs, re, &text, turns, Some(&stamps)) {
            Some(ps) => ps
                .iter()
                .map(|p| (p.start, p.end, p.text.split_whitespace().count()))
                .collect(),
            None => vec![(rs, re, idx.len())],
        };
        let segs: Vec<AlignableSegment> = parts
            .iter()
            .map(|&(s, e, _)| {
                let (a, b) = pad_trimmed(s, e);
                AlignableSegment::new(a, b, Channel::System)
            })
            .collect();
        let keys = align_turns_to_segments(turns, &segs, AlignMode::Call);
        if std::env::var("ONSET_DUMP").is_ok() {
            // Dump rows that contain a reference speaker change.
            let spk: Vec<&str> = idx.iter().map(|&i| words[i].speaker.as_str()).collect();
            if spk.windows(2).any(|w| w[0] != w[1]) {
                eprintln!(
                    "ROW {rs:.2}-{re:.2} parts {:?}",
                    parts
                        .iter()
                        .zip(&keys)
                        .map(|(p, k)| format!("{:.2}-{:.2} n{} ->{k}", p.0, p.1, p.2))
                        .collect::<Vec<_>>()
                );
                let mut last = "";
                for &i in idx.iter() {
                    if words[i].speaker != last {
                        eprintln!("   ref {} @ {:.2}", words[i].speaker, words[i].start);
                        last = &words[i].speaker;
                    }
                }
                for t in turns
                    .iter()
                    .filter(|t| t.end > rs - 1.0 && t.start < re + 1.0)
                {
                    eprintln!("   turn {:.2}-{:.2} {}", t.start, t.end, t.speaker);
                }
            }
        }
        let mut k = 0usize;
        for (pi, &(_, _, n)) in parts.iter().enumerate() {
            for _ in 0..n {
                if let Some(&wi) = idx.get(k) {
                    pred[wi] = Some(keys[pi].clone());
                }
                k += 1;
            }
        }
    }
    pred
}

fn score(words: &[Word], pred: &[Option<String>]) -> Score {
    let mut s = Score::default();
    let mut prev_turn_spk: Option<String> = None;
    let mut cur_spk: Option<String> = None;
    for (w, p) in words.iter().zip(pred) {
        if w.turn_pos == 0 {
            prev_turn_spk = cur_spk.clone();
            cur_spk = Some(w.speaker.clone());
        }
        let Some(p) = p else { continue };
        s.words += 1;
        let wrong = *p != w.speaker;
        s.wrong += wrong as usize;
        // Onset: early words of a turn that FOLLOWS another speaker.
        if prev_turn_spk.is_some() && w.turn_pos < ONSET_WORDS {
            s.onset += 1;
            s.onset_wrong += wrong as usize;
            s.onset_to_prev += (Some(p) == prev_turn_spk.as_ref()) as usize;
            if w.turn_pos == 0 {
                s.turns += 1;
                s.turns_first_wrong += wrong as usize;
            }
        }
    }
    s
}

#[test]
#[ignore = "real-file onset eval; run explicitly with --ignored"]
fn eval_onset_attribution() {
    let Some(dir) = eval_dir() else {
        eprintln!("SKIP: no zoom-samples dir");
        return;
    };
    let raw_labels = std::env::var("ONSET_RAW_LABELS").is_ok();
    let redemption: u32 = std::env::var("ONSET_REDEMPTION_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);
    let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    let mut total = Score::default();
    for e in entries {
        let d = e.path();
        if !d.is_dir() {
            continue;
        }
        let cache = d.join(".eval-cache");
        let Some(vtt) = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "vtt"))
        else {
            continue;
        };
        let Some(raw) = load_turns(&cache) else {
            continue;
        };
        let wav = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.to_string_lossy().ends_with(".16k-mono.wav"))
            .expect("cached wav");
        let cues = parse_vtt(&vtt);
        let words = cue_words(&cues);
        let turns = if raw_labels {
            raw
        } else {
            oracle_relabel(&raw, &cues)
        };
        let rows = vad_rows(&cache, &wav, redemption);
        let pred = production_labels(&rows, &words, &turns);
        let s = score(&words, &pred);
        eprintln!(
            "{:<20} words {:>6} wrong {:>5.1}% | onset words {:>5} wrong {:>5.1}% (to prev spk {:>5.1}%) | turns {:>4} first-word wrong {:>5.1}%",
            d.file_name().unwrap().to_string_lossy(),
            s.words,
            100.0 * s.wrong as f32 / s.words.max(1) as f32,
            s.onset,
            100.0 * s.onset_wrong as f32 / s.onset.max(1) as f32,
            100.0 * s.onset_to_prev as f32 / s.onset.max(1) as f32,
            s.turns,
            100.0 * s.turns_first_wrong as f32 / s.turns.max(1) as f32,
        );
        total.words += s.words;
        total.wrong += s.wrong;
        total.onset += s.onset;
        total.onset_wrong += s.onset_wrong;
        total.onset_to_prev += s.onset_to_prev;
        total.turns += s.turns;
        total.turns_first_wrong += s.turns_first_wrong;
    }
    eprintln!(
        "TOTAL wrong {:.1}% | onset wrong {:.1}% (to prev {:.1}%) | first-word wrong {:.1}%",
        100.0 * total.wrong as f32 / total.words.max(1) as f32,
        100.0 * total.onset_wrong as f32 / total.onset.max(1) as f32,
        100.0 * total.onset_to_prev as f32 / total.onset.max(1) as f32,
        100.0 * total.turns_first_wrong as f32 / total.turns.max(1) as f32,
    );
}

// ===========================================================================
// specs/0055 Stage 2 — a synthetic two-channel CALL built from the VTT samples.
//
// The busiest reference speaker plays the owner: their cue spans of the mixed
// recording go to `mic`, everything else to `system`. The `speakers` variant adds a
// delayed, attenuated copy of `system` to `mic` (speaker bleed: the remote voice
// reaching the mic through the room), which is what produces fake mic-dominant
// windows at the end of remote phrases. Rows are the app's live-VAD rows over the
// mix; each row's stored channel tag is the whole-row vote on the timeline, exactly
// as capture computes it. Truth is the VTT: owner words should be "local".
//
// OLD = owner turns from mic VAD + bleed guard, `plan_split` (owner turns only on
// mic rows), align. NEW = `plan_channel_split` on the timeline, then `plan_split`
// with remote turns inside the remote parts, align.
// ===========================================================================

use app_lib::diarization::align::LOCAL_SPEAKER_KEY;
use app_lib::diarization::channel_split::{
    is_single_side, plan_channel_split, ChannelTag, ChannelTimeline,
};
use app_lib::diarization::owner_turns::{
    filter_bleed_owner_segments, speech_segments_to_owner_turns,
};
use app_lib::diarization::segments::channel_from_db;
use app_lib::diarization::split::SplitPart;

const SR: usize = 16_000;

fn tag_str(t: Option<ChannelTag>) -> Option<String> {
    t.map(|t| {
        match t {
            ChannelTag::Microphone => "microphone",
            ChannelTag::System => "system",
            ChannelTag::Mixed => "mixed",
        }
        .to_string()
    })
}

fn words_of(idx: &[usize], words: &[Word]) -> (String, Vec<WordStamp>) {
    let text = idx
        .iter()
        .map(|&i| words[i].text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let stamps = idx
        .iter()
        .map(|&i| {
            serde_json::from_value(serde_json::json!({
                "w": words[i].text, "s": words[i].start, "e": words[i].end
            }))
            .unwrap()
        })
        .collect();
    (text, stamps)
}

fn rows_to_words(rows: &[(f32, f32)], words: &[Word]) -> Vec<Vec<usize>> {
    let mut row_words: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
    let mut r = 0usize;
    for (i, w) in words.iter().enumerate() {
        let mid = (w.start + w.end) / 2.0;
        while r < rows.len() && rows[r].1 < mid {
            r += 1;
        }
        if r < rows.len() && rows[r].0 <= mid {
            row_words[r].push(i);
        }
    }
    row_words
}

/// Align a row's final parts and write each part's key onto its words.
fn label_parts(
    parts: &[SplitPart],
    idx: &[usize],
    turns: &[SpeakerTurn],
    pred: &mut [Option<String>],
) {
    let segs: Vec<AlignableSegment> = parts
        .iter()
        .map(|p| {
            let (a, b) = pad_trimmed(p.start, p.end);
            AlignableSegment::new(a, b, channel_from_db(p.channel.as_deref()))
        })
        .collect();
    let keys = align_turns_to_segments(turns, &segs, AlignMode::Call);
    let mut k = 0usize;
    for (p, key) in parts.iter().zip(keys) {
        for _ in 0..p.text.split_whitespace().count() {
            if let Some(&wi) = idx.get(k) {
                pred[wi] = Some(key.clone());
            }
            k += 1;
        }
    }
}

fn whole(
    start: f32,
    end: f32,
    text: &str,
    channel: Option<String>,
    words: Vec<WordStamp>,
) -> SplitPart {
    SplitPart {
        start,
        end,
        text: text.to_string(),
        channel,
        words: Some(words),
    }
}

#[derive(Default)]
struct CallScore {
    words: usize,
    wrong: usize,
    owner_words: usize,
    owner_not_you: usize,
    remote_words: usize,
    remote_as_you: usize,
    owner_onsets: usize,
    owner_onset_wrong: usize,
    remote_onsets: usize,
    remote_onset_wrong: usize,
}

fn call_score(words: &[Word], pred: &[Option<String>], owner: &str) -> CallScore {
    let mut s = CallScore::default();
    let mut has_prev = false;
    for (w, p) in words.iter().zip(pred) {
        if w.turn_pos == 0 && s.words > 0 {
            has_prev = true;
        }
        let Some(p) = p else { continue };
        let is_owner = w.speaker == owner;
        let truth = if is_owner {
            LOCAL_SPEAKER_KEY
        } else {
            w.speaker.as_str()
        };
        let wrong = p != truth;
        s.words += 1;
        s.wrong += wrong as usize;
        if is_owner {
            s.owner_words += 1;
            s.owner_not_you += (p != LOCAL_SPEAKER_KEY) as usize;
        } else {
            s.remote_words += 1;
            s.remote_as_you += (p == LOCAL_SPEAKER_KEY) as usize;
        }
        if has_prev && w.turn_pos < ONSET_WORDS {
            if is_owner {
                s.owner_onsets += 1;
                s.owner_onset_wrong += wrong as usize;
            } else {
                s.remote_onsets += 1;
                s.remote_onset_wrong += wrong as usize;
            }
        }
    }
    s
}

fn pct(a: usize, b: usize) -> f32 {
    100.0 * a as f32 / b.max(1) as f32
}

#[test]
#[ignore = "real-file call eval; run explicitly with --ignored"]
fn eval_call_owner_onset_old_vs_new() {
    let Some(dir) = eval_dir() else {
        eprintln!("SKIP: no zoom-samples dir");
        return;
    };
    let no_words = std::env::var("ONSET_NO_WORDS").is_ok();
    let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let d = e.path();
        if !d.is_dir() {
            continue;
        }
        let cache = d.join(".eval-cache");
        let Some(vtt) = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "vtt"))
        else {
            continue;
        };
        let Some(raw) = load_turns(&cache) else {
            continue;
        };
        let wav = std::fs::read_dir(&cache)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.to_string_lossy().ends_with(".16k-mono.wav"))
            .expect("cached wav");
        let cues = parse_vtt(&vtt);
        let words = cue_words(&cues);
        let mut dur: HashMap<&str, f32> = HashMap::new();
        for c in &cues {
            *dur.entry(c.speaker.as_str()).or_default() += c.end - c.start;
        }
        let owner = dur
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(k, _)| k.to_string())
            .unwrap();
        let oracle = oracle_relabel(&raw, &cues);
        let remote: Vec<SpeakerTurn> = oracle
            .iter()
            .filter(|t| t.speaker != owner)
            .cloned()
            .collect();
        let rows = vad_rows(&cache, &wav, 400);
        let row_words = rows_to_words(&rows, &words);
        let mix = decode_audio_file(&wav).unwrap().to_whisper_format();

        for (variant, bleed_gain, bleed_delay_ms) in
            [("headphones", 0.0f32, 0usize), ("speakers", 0.7, 150)]
        {
            let mut mic = vec![0.0f32; mix.len()];
            let mut sys = mix.clone();
            for c in cues.iter().filter(|c| c.speaker == owner) {
                let a = ((c.start as f64) * SR as f64) as usize;
                let b = (((c.end as f64) * SR as f64) as usize).min(mix.len());
                for i in a.min(b)..b {
                    mic[i] = mix[i];
                    sys[i] = 0.0;
                }
            }
            if bleed_gain > 0.0 {
                let delay = bleed_delay_ms * SR / 1000;
                for i in delay..mic.len() {
                    mic[i] += bleed_gain * sys[i - delay];
                }
            }
            let timeline = ChannelTimeline::from_tracks(&mic, &sys, SR).unwrap();

            // OLD: owner turns from mic VAD + bleed guard (inject_owner_turns).
            let segs = get_speech_chunks(&mic, 2000).unwrap();
            let segs = filter_bleed_owner_segments(segs, &mic, &sys, SR);
            let mut all = remote.clone();
            all.extend(speech_segments_to_owner_turns(&segs));
            all.sort_by(|a, b| a.start.total_cmp(&b.start));

            let mut pred_old = vec![None; words.len()];
            let mut pred_new = vec![None; words.len()];
            for (ri, &(rs, re)) in rows.iter().enumerate() {
                let idx = &row_words[ri];
                if idx.is_empty() {
                    continue;
                }
                let (text, stamps) = words_of(idx, &words);
                // ONSET_NO_WORDS=1: rows without word stamps (Whisper, pre-Stage-2 live rows).
                let w = (!no_words).then_some(stamps.as_slice());
                let tag = tag_str(timeline.dominant_tag(rs, re));
                let is_mic = tag.as_deref() == Some("microphone");

                // OLD (split_straddling_rows, Call): mic rows split against every turn and
                // must keep an owner part; other rows only against remote turns.
                let row_turns = if is_mic { &all } else { &remote };
                let old = plan_split(rs, re, &text, row_turns, w)
                    .filter(|ps| {
                        !is_mic
                            || ps
                                .iter()
                                .any(|p| p.channel.as_deref() == Some("microphone"))
                    })
                    .unwrap_or_else(|| vec![whole(rs, re, &text, tag.clone(), stamps.clone())]);
                label_parts(&old, idx, &all, &mut pred_old);

                // NEW: channel cut first, then remote<->remote inside non-owner parts.
                let first = plan_channel_split(rs, re, &text, &timeline, w)
                    .unwrap_or_else(|| vec![whole(rs, re, &text, tag.clone(), stamps.clone())]);
                let mut fin: Vec<SplitPart> = Vec::new();
                for p in first {
                    // The turn split as the pipeline runs it after the channel cut.
                    let part_mic = p.channel.as_deref() == Some("microphone");
                    if part_mic && is_single_side(p.start, p.end, &timeline) {
                        fin.push(p);
                        continue;
                    }
                    let pt = if part_mic { &all } else { &remote };
                    match plan_split(p.start, p.end, &p.text, pt, p.words.as_deref()).filter(|ps| {
                        !part_mic
                            || ps
                                .iter()
                                .any(|x| x.channel.as_deref() == Some("microphone"))
                    }) {
                        Some(sub) => fin.extend(sub),
                        None => fin.push(p),
                    }
                }
                label_parts(&fin, idx, &all, &mut pred_new);
            }
            for (name, pred) in [("OLD", &pred_old), ("NEW", &pred_new)] {
                let s = call_score(&words, pred, &owner);
                eprintln!(
                    "{:<20} {:<10} {} | wrong {:>5.1}% | owner not-You {:>5.1}% | remote as-You {:>5.1}% | owner-onset wrong {:>5.1}% (n={}) | remote-onset wrong {:>5.1}% (n={})",
                    d.file_name().unwrap().to_string_lossy(),
                    variant,
                    name,
                    pct(s.wrong, s.words),
                    pct(s.owner_not_you, s.owner_words),
                    pct(s.remote_as_you, s.remote_words),
                    pct(s.owner_onset_wrong, s.owner_onsets),
                    s.owner_onsets,
                    pct(s.remote_onset_wrong, s.remote_onsets),
                    s.remote_onsets,
                );
            }
        }
    }
}

/// The real two-channel capture: does the offline timeline reproduce the stored row
/// tags, how many rows does Stage 2 cut, and does an Opus round-trip (specs/0072 —
/// kept audio is compressed) change the cuts? Set `ONSET_CALL_FOLDER` (mic.wav +
/// system.wav + transcripts.json) and `ONSET_OPUS_DIR` (mic.opus + system.opus).
#[test]
#[ignore = "real two-channel capture; run explicitly with --ignored"]
fn eval_real_capture_channel_cuts() {
    let (Ok(folder), Ok(opus)) = (
        std::env::var("ONSET_CALL_FOLDER"),
        std::env::var("ONSET_OPUS_DIR"),
    ) else {
        eprintln!("SKIP: set ONSET_CALL_FOLDER and ONSET_OPUS_DIR");
        return;
    };
    let (folder, opus) = (PathBuf::from(folder), PathBuf::from(opus));
    let dec = |p: PathBuf| decode_audio_file(&p).unwrap().to_whisper_format();
    let wav_tl = ChannelTimeline::from_tracks(
        &dec(folder.join("mic.wav")),
        &dec(folder.join("system.wav")),
        SR,
    )
    .unwrap();
    let opus_tl = ChannelTimeline::from_tracks(
        &dec(opus.join("mic.opus")),
        &dec(opus.join("system.opus")),
        SR,
    )
    .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.join("transcripts.json")).unwrap())
            .unwrap();
    let (mut rows, mut tag_match, mut cut_wav, mut cut_opus, mut same_cut) = (0, 0, 0, 0, 0);
    let (mut owner_parts, mut parts_total) = (0, 0);
    let num = |x: &serde_json::Value| x.as_f64().or_else(|| x.as_str()?.parse().ok());
    for s in v["segments"].as_array().unwrap() {
        let (Some(a), Some(b)) = (num(&s["audio_start_time"]), num(&s["audio_end_time"])) else {
            continue;
        };
        let (a, b) = (a as f32, b as f32);
        let text = s["text"].as_str().unwrap_or("");
        rows += 1;
        tag_match +=
            (tag_str(wav_tl.dominant_tag(a, b)).as_deref() == s["channel"].as_str()) as usize;
        let pw = plan_channel_split(a, b, text, &wav_tl, None);
        let po = plan_channel_split(a, b, text, &opus_tl, None);
        cut_wav += pw.is_some() as usize;
        cut_opus += po.is_some() as usize;
        let sig = |p: &Option<Vec<SplitPart>>| {
            p.as_ref().map(|ps| {
                ps.iter()
                    .map(|x| (x.text.clone(), x.channel.clone()))
                    .collect::<Vec<_>>()
            })
        };
        same_cut += (sig(&pw) == sig(&po)) as usize;
        if let Some(ps) = &pw {
            parts_total += ps.len();
            owner_parts += ps
                .iter()
                .filter(|p| p.channel.as_deref() == Some("microphone"))
                .count();
        }
    }
    eprintln!(
        "rows {rows} | offline tag == stored tag {tag_match} | cut (wav) {cut_wav} -> {parts_total} parts, {owner_parts} owner | cut (opus) {cut_opus} | identical cut wav vs opus {same_cut}/{rows}"
    );
}
