//! specs/0072 W2: every channel reader works on a meeting whose kept channels the lifecycle
//! compressed to Opus (`mic.opus` / `system.opus`).
//!
//! Fixtures are `say` speech written as 16 kHz mono WAVs (the capture format), compressed by
//! the production compressor with the bundled ffmpeg sidecar. The readers decode through the
//! app's own `decode_audio_file`, which finds ffmpeg on `PATH`. Each test SKIPS cleanly when
//! `say`, the sidecar or an ffmpeg on `PATH` is missing (never triggering ffmpeg's
//! auto-download from a test).

mod common;

use std::path::{Path, PathBuf};

use app_lib::audio::channel_writer::{mic_channel_path, system_channel_path};
use app_lib::audio::lifecycle::compress::compress_channels;

/// The bundled sidecar (what the app ships), used to compress like production does.
fn sidecar() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("ffmpeg-"))
        })
}

/// A meeting folder with `mic.wav` (speech) and `system.wav` (different speech), 16 kHz mono,
/// plus the ffmpeg to compress it with. `None` = skip.
fn speech_folder(test: &str) -> Option<(tempfile::TempDir, PathBuf)> {
    if which::which("ffmpeg").is_err() {
        eprintln!("SKIP {test}: no ffmpeg on PATH for the app's decoder");
        return None;
    }
    let Some(ff) = sidecar() else {
        eprintln!("SKIP {test}: no bundled ffmpeg sidecar in binaries/");
        return None;
    };
    let dir = tempfile::tempdir().unwrap();
    let mic = "the owner of this meeting is speaking into the microphone right now";
    let system = "and this is the remote side of the call coming through the speakers";
    if !common::synth_say_wav(mic, &dir.path().join("mic.wav"))
        || !common::synth_say_wav(system, &dir.path().join("system.wav"))
    {
        eprintln!("SKIP {test}: `say` unavailable");
        return None;
    }
    Some((dir, ff))
}

fn seconds(path: &Path) -> f64 {
    let d = app_lib::audio::decoder::decode_audio_file(path).unwrap();
    d.to_whisper_format().len() as f64 / 16_000.0
}

/// Spec task 16 / sabotage #6: `.opus` decodes through the app's decoder, 16 kHz mono, with
/// the WAV's sample count to within 1%.
#[test]
fn a_compressed_channel_decodes_like_its_wav() {
    let Some((dir, ff)) = speech_folder("a_compressed_channel_decodes_like_its_wav") else {
        return;
    };
    let wav = dir.path().join("mic.wav");
    let want = app_lib::audio::decoder::decode_audio_file(&wav)
        .unwrap()
        .to_whisper_format()
        .len();
    compress_channels(&ff, dir.path()).unwrap();
    let opus = dir.path().join("mic.opus");
    assert!(opus.exists() && !wav.exists());

    let decoded = app_lib::audio::decoder::decode_audio_file(&opus).unwrap();
    assert_eq!(
        (decoded.sample_rate, decoded.channels),
        (16_000, 1),
        "decoded straight to the channel format"
    );
    let got = decoded.to_whisper_format().len();
    assert!(
        got.abs_diff(want) * 100 <= want,
        "opus decodes to {got} samples, the WAV had {want}"
    );
}

/// The owner track (spec 0046/0047) is read from `mic.opus` once compressed.
#[tokio::test]
async fn owner_turns_come_from_a_compressed_mic_channel() {
    let Some((dir, ff)) = speech_folder("owner_turns_come_from_a_compressed_mic_channel") else {
        return;
    };
    // A quiet remote line, so the bleed guard sees the owner's mic dominate.
    let secs = seconds(&dir.path().join("mic.wav"));
    let quiet = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!(
            "sine=frequency=200:duration={secs}:sample_rate=16000"
        ))
        .args(["-af", "volume=0.01", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(dir.path().join("system.wav"))
        .status()
        .unwrap();
    assert!(quiet.success());
    compress_channels(&ff, dir.path()).unwrap();
    assert_eq!(
        mic_channel_path(dir.path()),
        Some(dir.path().join("mic.opus"))
    );

    let app = tauri::test::mock_app();
    let turns =
        app_lib::diarization::owner_turns::owner_turns_for_meeting(app.handle(), dir.path()).await;
    assert!(!turns.is_empty(), "the owner's speech became turns");
}

/// Retranscription channel tags (the deferred "You" attribution) read compressed channels.
#[test]
fn retranscription_channel_tags_read_compressed_channels() {
    let Some((dir, ff)) = speech_folder("retranscription_channel_tags_read_compressed_channels")
    else {
        return;
    };
    let expected = seconds(&dir.path().join("mic.wav"));
    compress_channels(&ff, dir.path()).unwrap();
    assert_eq!(
        system_channel_path(dir.path()),
        Some(dir.path().join("system.opus"))
    );
    let profile =
        app_lib::audio::retranscription_channels::ChannelRmsProfile::load(dir.path(), expected);
    assert!(profile.is_some(), "the compressed channels were loaded");
}

/// Spec task 18: a folder with channels but no mix (an old "delete immediately" meeting
/// processed later) is transcribed from both sides mixed, never from one channel.
#[test]
fn a_channels_only_folder_is_transcribed_from_both_sides_mixed() {
    let Some((dir, ff)) =
        speech_folder("a_channels_only_folder_is_transcribed_from_both_sides_mixed")
    else {
        return;
    };
    let longest = seconds(&dir.path().join("mic.wav")).max(seconds(&dir.path().join("system.wav")));
    compress_channels(&ff, dir.path()).unwrap();

    let audio =
        app_lib::audio::meeting_audio::find_audio_file_with(dir.path(), || Some(ff.clone()))
            .unwrap();
    let file = audio
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        file.starts_with(".nixon_decode_mix_"),
        "a temporary mix, not a channel file: {file}"
    );
    let mixed = seconds(&audio.path);
    assert!(
        (mixed - longest).abs() < 0.1,
        "mix {mixed:.2}s, longest side {longest:.2}s"
    );
    let samples = |p: &Path| {
        let d = app_lib::audio::decoder::decode_audio_file(p).unwrap();
        d.to_whisper_format()
    };
    let mix = samples(&audio.path);
    for side in ["mic.opus", "system.opus"] {
        let s = samples(&dir.path().join(side));
        let n = s.len().min(mix.len());
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let corr =
            dot(&mix[..n], &s[..n]) / (dot(&mix[..n], &mix[..n]) * dot(&s[..n], &s[..n])).sqrt();
        assert!(corr > 0.3, "{side} is in the mix (correlation {corr:.2})");
    }

    let temp = audio.path.clone();
    drop(audio);
    assert!(!temp.exists(), "the temporary mix is removed after use");
    assert!(dir.path().join("mic.opus").exists() && dir.path().join("system.opus").exists());
}
