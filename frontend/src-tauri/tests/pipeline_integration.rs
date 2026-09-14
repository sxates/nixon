//! Pipeline integration test (specs/0009, priority #3): "recording works, minus
//! the mic".
//!
//! Pushes fixture `AudioChunk`s through the REAL `AudioPipeline` (the same mix ->
//! resample -> VAD path used during a live recording) WITHOUT the Core Audio /
//! mic hardware layer, and asserts that VAD-filtered 16 kHz segments reach the
//! transcription stage. When the Parakeet model is present, it additionally runs
//! those segments through the real engine and asserts a non-empty transcript —
//! i.e. the full capture->transcript path minus the hardware.
//!
//! Injection point: `AudioPipeline::new(receiver, transcription_sender, ..)`.
//! We own both channel ends, so we feed the receiver and observe the sender.
//!
//! Run with:
//!   cargo test --features metal --test pipeline_integration -- --nocapture

mod common;

use app_lib::audio::audio_processing::SPECTRUM_BANDS;
use app_lib::audio::device_detection::InputDeviceKind;
use app_lib::audio::pipeline::{AudioPipeline, ChannelTag, TranscriptionChunk};
use app_lib::audio::recording_state::{AudioChunk, DeviceType, RecordingState};
use app_lib::parakeet_engine::ParakeetEngine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

const CAPTURE_RATE: u32 = 48_000; // Core Audio / mic capture rate the pipeline expects.

/// A live UI event captured from the pipeline's emitter (specs/0029 WS4.2):
/// `(event_name, payload)`.
type LiveEvent = (String, serde_json::Value);

/// Collected live events. Process-global because the pipeline's emitter registry is
/// once-per-process (mirroring how the app registers its AppHandle emitter at the first
/// recording start).
static LIVE_EVENTS: OnceLock<Mutex<Vec<LiveEvent>>> = OnceLock::new();

/// Serializes pipeline runs within this test binary so collected live events can be
/// attributed to a single run (tests share the process-global emitter above).
static PIPELINE_RUN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn install_live_event_collector() -> &'static Mutex<Vec<LiveEvent>> {
    let store = LIVE_EVENTS.get_or_init(|| Mutex::new(Vec::new()));
    app_lib::audio::pipeline::register_live_event_emitter(|event, payload| {
        if let Some(store) = LIVE_EVENTS.get() {
            store
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((event.to_string(), payload));
        }
    });
    store
}

/// Drive the real AudioPipeline with a fixture. Returns the VAD segments it emits to
/// the transcription stage (16 kHz mono chunks), the live UI events
/// (`recording-level` / `recording-spectrum`) emitted during this run (specs/0029
/// WS4.2), and the pre-mixed recording chunks delivered to the save path.
///
/// `live_transcription` = false exercises record-only mode (specs/0029 WS7.2): the
/// pipeline must skip the VAD/STT stage entirely while still mixing/saving audio and
/// driving the live spectrum feed.
async fn run_pipeline_with_options(
    samples_48k: Vec<f32>,
    live_transcription: bool,
) -> (Vec<TranscriptionChunk>, Vec<LiveEvent>, Vec<AudioChunk>) {
    // Hold the run lock for the whole run so the collected live events belong to this
    // run only.
    let _run_guard = PIPELINE_RUN_LOCK.lock().await;
    let store = install_live_event_collector();
    let events_baseline = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len();

    // Bounded channels mirror production (specs/0028): capacity is large enough
    // that this short fixture never hits the drop-newest backpressure path.
    const CHANNEL_CAP: usize = 8192;
    // Channel into the pipeline (would normally come from capture).
    let (audio_tx, audio_rx) = mpsc::channel::<AudioChunk>(CHANNEL_CAP);
    // Channel out of the pipeline toward transcription (specs/0029 WS3.4: carries
    // the audio chunk plus its capture-channel tag).
    let (trans_tx, mut trans_rx) = mpsc::channel::<TranscriptionChunk>(CHANNEL_CAP);
    // Channel out of the pipeline toward the recording saver (pre-mixed audio).
    let (rec_tx, mut rec_rx) = mpsc::unbounded_channel::<AudioChunk>();

    let state = RecordingState::new();

    let mut pipeline = AudioPipeline::new(
        audio_rx,
        trans_tx,
        state,
        100, // target_chunk_duration_ms (ignored: VAD controls segmentation)
        CAPTURE_RATE,
        "Test Microphone".to_string(),
        InputDeviceKind::Wired,
        "Test System".to_string(),
        InputDeviceKind::Wired,
        Arc::new(AtomicBool::new(live_transcription)),
    )
    .expect("construct AudioPipeline");
    pipeline.set_recording_sender(Some(rec_tx));

    let handle = tokio::spawn(pipeline.run());

    // Feed the speech as a sequence of mic chunks (mimics streamed capture).
    // ~20ms chunks at 48 kHz.
    let chunk_len = (CAPTURE_RATE as usize / 50).max(1);
    let mut timestamp = 0.0f64;
    for (chunk_id, chunk) in samples_48k.chunks(chunk_len).enumerate() {
        audio_tx
            .send(AudioChunk {
                data: chunk.to_vec(),
                sample_rate: CAPTURE_RATE,
                timestamp,
                chunk_id: chunk_id as u64,
                device_type: DeviceType::Microphone,
            })
            .await
            .expect("send chunk into pipeline");
        timestamp += chunk.len() as f64 / CAPTURE_RATE as f64;
    }

    // Closing the sender lets the pipeline drain, flush VAD, and exit run().
    drop(audio_tx);
    handle
        .await
        .expect("pipeline task join")
        .expect("pipeline run ok");

    let mut out = Vec::new();
    while let Ok(chunk) = trans_rx.try_recv() {
        out.push(chunk);
    }
    let mut recorded = Vec::new();
    while let Ok(chunk) = rec_rx.try_recv() {
        recorded.push(chunk);
    }

    let events = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)[events_baseline..]
        .to_vec();
    (out, events, recorded)
}

/// Classic live-transcription run (the default path used by most tests).
async fn run_pipeline_with(samples_48k: Vec<f32>) -> (Vec<TranscriptionChunk>, Vec<LiveEvent>) {
    let (segments, events, _recorded) = run_pipeline_with_options(samples_48k, true).await;
    (segments, events)
}

#[tokio::test]
async fn pipeline_emits_vad_segments_for_speech() {
    let Some(speech_16k) = common::speech_samples_16k() else {
        eprintln!("SKIP pipeline_emits_vad_segments_for_speech: `say` unavailable");
        return;
    };

    // The pipeline expects capture-rate (48 kHz) audio; upsample the 16 kHz fixture.
    let speech_48k = common::resample_linear(&speech_16k, 16_000, CAPTURE_RATE);

    let (segments, _live_events) = run_pipeline_with(speech_48k).await;

    assert!(
        !segments.is_empty(),
        "pipeline emitted NO transcription segments for spoken audio \
         (recording-minus-mic path is broken)"
    );
    // The pipeline always emits 16 kHz mono segments toward the STT engine.
    for seg in &segments {
        assert_eq!(
            seg.chunk.sample_rate, 16_000,
            "segments to STT must be 16 kHz"
        );
        assert!(
            seg.chunk.data.len() >= 800,
            "segment below 50ms min should be dropped"
        );
        // specs/0029 WS3.4: this fixture feeds ONLY the microphone track (system is
        // silent), so no segment may be attributed to the system channel or Mixed.
        assert!(
            matches!(seg.channel, Some(ChannelTag::Microphone) | None),
            "mic-only fixture produced a non-microphone channel tag: {:?}",
            seg.channel
        );
    }
    // And the speech itself must be positively attributed to the mic track.
    assert!(
        segments
            .iter()
            .any(|s| s.channel == Some(ChannelTag::Microphone)),
        "mic-only speech fixture yielded no microphone-tagged segment (WS3.4 channel \
         attribution is not reaching the transcription stage)"
    );

    // Bonus: if the model is present, the emitted segments should transcribe.
    if let Some(models_dir) = common::find_parakeet_models_dir() {
        let engine =
            ParakeetEngine::new_with_models_dir(Some(models_dir)).expect("construct engine");
        engine.discover_models().await.expect("discover");
        engine
            .load_model(common::DEFAULT_PARAKEET_MODEL)
            .await
            .expect("load model");

        let mut full = String::new();
        for seg in segments {
            let text = engine
                .transcribe_audio(seg.chunk.data)
                .await
                .expect("transcribe pipeline segment");
            full.push(' ');
            full.push_str(&text);
        }
        eprintln!("End-to-end (minus mic) transcript: {full:?}");
        assert!(
            !full.trim().is_empty(),
            "end-to-end pipeline produced an empty transcript for known speech"
        );
        common::assert_transcript_recovers_words(&full, 2);
    } else {
        common::skip_no_model("pipeline_emits_vad_segments_for_speech (transcription assertion)");
    }
}

#[tokio::test]
async fn pipeline_emits_nothing_for_silence() {
    // 3 seconds of pure silence at capture rate -> VAD should gate it all out,
    // so no segments reach the transcription stage.
    let silence = common::silence(3.0, CAPTURE_RATE);
    let (segments, _live_events) = run_pipeline_with(silence).await;
    assert!(
        segments.is_empty(),
        "pipeline emitted {} segment(s) for pure silence (should gate all silence out)",
        segments.len()
    );
}

/// specs/0029 WS4.2: the live spectrum feed must be driven by the RAW mixed-window
/// path, NOT the VAD-gated transcription path. Pure silence produces ZERO transcription
/// segments (VAD gates everything out) yet must still produce `recording-spectrum` /
/// `recording-level` events (with flat bands) — the spectrometer only goes quiet when
/// audio stops FLOWING, not when speech stops.
#[tokio::test]
async fn spectrum_events_emitted_for_silence_independent_of_vad() {
    let silence = common::silence(3.0, CAPTURE_RATE);
    let (segments, events) = run_pipeline_with(silence).await;

    assert!(
        segments.is_empty(),
        "precondition: silence must not reach transcription (VAD-gated)"
    );

    let spectra: Vec<&serde_json::Value> = events
        .iter()
        .filter(|(name, _)| name == "recording-spectrum")
        .map(|(_, payload)| payload)
        .collect();
    assert!(
        !spectra.is_empty(),
        "recording-spectrum must be emitted from the raw mixed path even when VAD emits \
         nothing (the spectrometer must not be VAD-gated — 0029 WS4.2)"
    );
    for payload in &spectra {
        // Exact frontend contract (useRecordingWaveform): { bands: number[] in 0..1 }.
        let bands = payload
            .get("bands")
            .and_then(|b| b.as_array())
            .expect("recording-spectrum payload must be { bands: [...] }");
        assert_eq!(
            bands.len(),
            SPECTRUM_BANDS,
            "always emits the full band count"
        );
        assert!(
            bands.iter().all(|b| b.as_f64() == Some(0.0)),
            "a silent window must produce flat (all-zero) bands, got {bands:?}"
        );
    }

    let level = events
        .iter()
        .find(|(name, _)| name == "recording-level")
        .map(|(_, payload)| payload)
        .expect("recording-level must be emitted alongside the spectrum from the mixed path");
    // Exact frontend contract: { rms: number, peak: number } in 0..1.
    assert_eq!(level.get("rms").and_then(|v| v.as_f64()), Some(0.0));
    assert_eq!(level.get("peak").and_then(|v| v.as_f64()), Some(0.0));
    // specs/0057 §3.2: the two record-page needles read the CLEAN pre-mix channels.
    for ch in ["mic", "sys"] {
        assert_eq!(level.pointer(&format!("/{ch}/rms")).and_then(|v| v.as_f64()), Some(0.0));
        assert_eq!(level.pointer(&format!("/{ch}/peak")).and_then(|v| v.as_f64()), Some(0.0));
    }
}

/// specs/0029 WS7.2: record-only mode. With live transcription DISABLED, real speech
/// must produce ZERO transcription sends (the whole VAD/STT stage is skipped — this is
/// the CPU saving) while the mixed audio still reaches the recording/save path and the
/// live spectrum/level feed still animates (its emitter is registered at app setup,
/// independent of the transcription worker).
#[tokio::test]
async fn record_only_mode_saves_audio_without_transcription_sends() {
    let Some(speech_16k) = common::speech_samples_16k() else {
        eprintln!(
            "SKIP record_only_mode_saves_audio_without_transcription_sends: `say` unavailable"
        );
        return;
    };
    let speech_48k = common::resample_linear(&speech_16k, 16_000, CAPTURE_RATE);
    let fed_samples = speech_48k.len();

    let (segments, events, recorded) = run_pipeline_with_options(speech_48k, false).await;

    // The STT feed must be fully dark — not merely gated at the send.
    assert!(
        segments.is_empty(),
        "record-only mode leaked {} transcription segment(s) for spoken audio",
        segments.len()
    );

    // The recording/save path must still receive the mixed windows (audio is saved).
    assert!(
        !recorded.is_empty(),
        "record-only mode delivered NO mixed audio to the recording path (audio would be lost)"
    );
    let recorded_samples: usize = recorded.iter().map(|c| c.data.len()).sum();
    // Mixed windows are fixed-size (600 ms), so the tail may be padded/held back;
    // require that at least half of the fed audio made it through to the saver.
    assert!(
        recorded_samples >= fed_samples / 2,
        "record-only mode recorded only {recorded_samples} of {fed_samples} fed samples"
    );
    for chunk in &recorded {
        assert_eq!(
            chunk.sample_rate, CAPTURE_RATE,
            "recording stays at capture rate"
        );
    }

    // The spectrometer must keep animating (specs/0029 WS4.2 x WS7.2).
    assert!(
        events.iter().any(|(name, _)| name == "recording-spectrum"),
        "record-only mode must still emit recording-spectrum from the mixed path"
    );
    assert!(
        events.iter().any(|(name, _)| name == "recording-level"),
        "record-only mode must still emit recording-level from the mixed path"
    );
}

/// low-power-mode spec §3: mid-recording live-transcription toggle. Drives ONE running
/// pipeline through three phases against the shared `Arc<AtomicBool>` flag the
/// `api_apply_live_transcription_now` command flips, proving `SttStage::sync()` attaches
/// AND detaches the VAD/STT stage mid-stream:
///   1. flag=false: speech in → ZERO transcription chunks (stage skipped).
///   2. flag=true : same speech in → the stage attaches, ≥1 chunk arrives.
///   3. flag=false: same speech in → the stage detaches. spec 0051 WS1: detaching now
///      flushes the VAD's still-open tail instead of dropping it, so AT MOST one
///      flushed chunk is expected here (phase 2's speech may not have closed out
///      naturally before the toggle) — anything beyond that would mean phase 3's
///      freshly-fed audio wrongly reached the (detached) VAD/STT stage.
#[tokio::test]
async fn mid_stream_live_toggle_attaches_and_detaches_stt() {
    let Some(speech_16k) = common::speech_samples_16k() else {
        eprintln!("SKIP mid_stream_live_toggle_attaches_and_detaches_stt: `say` unavailable");
        return;
    };
    let speech_48k = common::resample_linear(&speech_16k, 16_000, CAPTURE_RATE);

    // Serialize against the other pipeline runs (shared process-global emitter).
    let _run_guard = PIPELINE_RUN_LOCK.lock().await;

    const CHANNEL_CAP: usize = 8192;
    let (audio_tx, audio_rx) = mpsc::channel::<AudioChunk>(CHANNEL_CAP);
    let (trans_tx, mut trans_rx) = mpsc::channel::<TranscriptionChunk>(CHANNEL_CAP);

    let state = RecordingState::new();

    // The flag starts DEFERRED (false) — exactly as a deferred session's pipeline is built.
    let live_stt = Arc::new(AtomicBool::new(false));

    let pipeline = AudioPipeline::new(
        audio_rx,
        trans_tx,
        state,
        100,
        CAPTURE_RATE,
        "Test Microphone".to_string(),
        InputDeviceKind::Wired,
        "Test System".to_string(),
        InputDeviceKind::Wired,
        live_stt.clone(),
    )
    .expect("construct AudioPipeline");

    let handle = tokio::spawn(pipeline.run());

    // ~20ms mic chunks at 48 kHz (mirrors the shared harness's streamed feed).
    let chunk_len = (CAPTURE_RATE as usize / 50).max(1);

    // Phase 1: deferred. Speech must produce ZERO transcription chunks.
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let phase1 = common::drain_settled(&mut trans_rx).await;
    assert!(
        phase1.is_empty(),
        "deferred phase leaked {} transcription chunk(s) before go-live",
        phase1.len()
    );

    // Phase 2: go live mid-recording. The stage must attach and emit segments.
    live_stt.store(true, Ordering::SeqCst);
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let phase2 = common::drain_settled(&mut trans_rx).await;
    assert!(
        !phase2.is_empty(),
        "mid-recording go-live did not attach the VAD/STT stage (no chunks after enabling)"
    );

    // Phase 3: go deferred again. The stage must detach, emitting at most the one-time
    // detach-flush tail (spec 0051 WS1) — no unbounded new segmentation from phase 3's
    // freshly-fed audio, which the (now detached) VAD never sees.
    live_stt.store(false, Ordering::SeqCst);
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let phase3 = common::drain_settled(&mut trans_rx).await;
    assert!(
        phase3.len() <= 1,
        "mid-recording go-defer did not detach the VAD/STT stage ({} chunk(s) after disabling, \
         expected at most the one-time detach-flush tail)",
        phase3.len()
    );

    // Phase 3b: still deferred, feed AGAIN. The one-time detach-flush tail from phase 3
    // is spent — a genuinely detached stage must be silent here. `phase3.len() <= 1`
    // alone can't tell "legitimately detached, flushed its one tail" apart from "sync()
    // never detached at all, and this is just the still-attached VAD closing phase 2's
    // utterance at the phase-2/phase-3 concatenation boundary" — both produce exactly
    // one chunk in phase 3. A second deferred pass discriminates them: only the latter
    // keeps producing chunks.
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let phase3b = common::drain_settled(&mut trans_rx).await;
    assert!(
        phase3b.is_empty(),
        "detached stage still transcribing: {} chunk(s) (sync() did not actually detach)",
        phase3b.len()
    );

    drop(audio_tx);
    handle
        .await
        .expect("pipeline task join")
        .expect("pipeline run ok");
}

/// spec 0051 WS1: a live → defer → live session must produce a MONOTONIC transcript
/// timeline. Before the fix the re-attached VAD restarted at 0, so phase-3 chunks
/// carried timestamps near zero and the frontend's chunk_start_time sort inserted
/// them at the TOP of the transcript.
#[tokio::test]
async fn live_defer_live_keeps_timestamps_monotonic() {
    let Some(speech_16k) = common::speech_samples_16k() else {
        eprintln!("SKIP live_defer_live_keeps_timestamps_monotonic: `say` unavailable");
        return;
    };
    let speech_48k = common::resample_linear(&speech_16k, 16_000, CAPTURE_RATE);

    let _run_guard = PIPELINE_RUN_LOCK.lock().await;

    const CHANNEL_CAP: usize = 8192;
    let (audio_tx, audio_rx) = mpsc::channel::<AudioChunk>(CHANNEL_CAP);
    let (trans_tx, mut trans_rx) = mpsc::channel::<TranscriptionChunk>(CHANNEL_CAP);

    let state = RecordingState::new();
    // Starts LIVE — this is the owner's actual scenario (a live meeting toggled to
    // deferred and back), not the low-power start-deferred one.
    let live_stt = Arc::new(AtomicBool::new(true));

    let pipeline = AudioPipeline::new(
        audio_rx,
        trans_tx,
        state,
        100,
        CAPTURE_RATE,
        "Test Microphone".to_string(),
        InputDeviceKind::Wired,
        "Test System".to_string(),
        InputDeviceKind::Wired,
        live_stt.clone(),
    )
    .expect("construct AudioPipeline");

    let handle = tokio::spawn(pipeline.run());

    let chunk_len = (CAPTURE_RATE as usize / 50).max(1);

    // Phase 1: live. Collect the baseline timeline. One ~2.9s utterance may not carry
    // ~400ms of trailing silence on its own, so the VAD's redemption window can still be
    // open (uncommitted) at the end of this phase — the segment doesn't necessarily
    // close out here. That's fine: the flush-on-detach in phase 2 (spec 0051 WS1) is
    // exactly what guarantees it isn't lost, so the true phase-1 baseline is whatever
    // phase 1 emits directly PLUS phase 2's detach-flush tail (still phase-1 audio,
    // merely delivered late).
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let phase1 = common::drain_settled(&mut trans_rx).await;

    // Phase 2: deferred. Speech in, nothing NEW out — but the detach flushes whatever
    // phase 1 left buffered in the VAD (its tail may not have closed out naturally).
    live_stt.store(false, Ordering::SeqCst);
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let phase2_tail = common::drain_settled(&mut trans_rx).await;

    let phase1_baseline: Vec<&TranscriptionChunk> =
        phase1.iter().chain(phase2_tail.iter()).collect();
    assert!(
        !phase1_baseline.is_empty(),
        "live phase (plus its phase-2 detach flush) must produce SOME transcription chunks"
    );
    let phase1_max = phase1_baseline
        .iter()
        .map(|c| c.chunk.timestamp)
        .fold(f64::MIN, f64::max);

    // Phase 3: live again. THIS is the regression: these must land AFTER phase 1. As
    // with phase 1, this single utterance may still be open (uncommitted) at the end of
    // the drain — stopping the pipeline forces the final flush (`flush_remaining_audio`),
    // which is where its tail is guaranteed to land, so the phase-3 result is whatever
    // the live drain caught PLUS whatever the stop-time flush emits.
    live_stt.store(true, Ordering::SeqCst);
    common::feed(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE).await;
    let mut phase3 = common::drain_settled(&mut trans_rx).await;

    drop(audio_tx);
    handle
        .await
        .expect("pipeline task join")
        .expect("pipeline run ok");
    while let Ok(chunk) = trans_rx.try_recv() {
        phase3.push(chunk);
    }

    assert!(!phase3.is_empty(), "re-attached stage must produce chunks again");

    // `phase1_max` alone is not a reliable floor: when phase 1 emits nothing directly
    // (its segment stayed open until the phase-2 detach flush), `phase1_max` collapses
    // to whatever that single flushed tail happened to be timestamped at — which even
    // an UN-fixed, un-offset re-attached VAD (restarting its own clock near 0) can
    // trivially clear. The floor that actually pins down the session position is known
    // a priori from how much audio was fed before phase 3 ever started, independent of
    // what got emitted: phase 1 + phase 2 each fed one full fixture. Subtract one mix
    // window (600ms) for slack, since a segment's start timestamp can precede the
    // session position by up to one window.
    let fixture_secs = speech_16k.len() as f64 / 16_000.0;
    let resume_floor = 2.0 * fixture_secs - 0.6;

    for chunk in &phase3 {
        assert!(
            chunk.chunk.timestamp > phase1_max,
            "post-resume segment at {:.3}s must follow the phase-1 timeline (max {:.3}s) — \
             a re-attached VAD restarting at 0 is the 0051 bug",
            chunk.chunk.timestamp,
            phase1_max
        );
        assert!(
            chunk.chunk.timestamp >= resume_floor,
            "post-resume segment at {:.3}s must land at/after the session position phase 3 \
             resumed at ({:.3}s of audio fed) — a re-attached VAD restarting its OWN clock \
             near 0 is the 0051 bug this test is named for",
            chunk.chunk.timestamp,
            resume_floor
        );
    }
}

/// spec 0051 final review, Finding 4 — the OTHER half of WS1: channel attribution across
/// the toggle. The spec's acceptance criteria say "Resumed-tail segments carry correct
/// mic/system channel attribution", and WS1 moved the channel-window clock onto the new
/// session-audio base (`recorded_ms`) while segment timestamps moved onto the same base
/// via `SttStage::offset_ms`. Nothing asserted `chunk.channel` on any toggled path: the
/// only channel assertions lived in the no-toggle speech test, so desyncing the two
/// clocks left the whole suite green. This is the input 0046/0047 owner-boundary
/// diarization is built on.
///
/// Live (mic-only speech) → deferred → live again (SYSTEM-only speech). The phase-3
/// segments must be attributed to the SYSTEM channel: their spans are resolved against
/// `channel_windows`, which is cleared on attach and refilled on the session clock. Any
/// drift between the two clocks makes the lookup miss entirely and the tag collapses to
/// `None`.
#[tokio::test]
async fn live_defer_live_preserves_channel_attribution() {
    let Some(speech_16k) = common::speech_samples_16k() else {
        eprintln!("SKIP live_defer_live_preserves_channel_attribution: `say` unavailable");
        return;
    };
    let speech_48k = common::resample_linear(&speech_16k, 16_000, CAPTURE_RATE);
    // A trailing silence pad ends each mic phase on a window boundary's worth of silence,
    // so the mic remainder still sitting in the ring buffer when phase 3 starts is SILENT
    // and can't colour the first system-only window.
    let pad_48k = common::silence(1.5, CAPTURE_RATE);

    let _run_guard = PIPELINE_RUN_LOCK.lock().await;

    const CHANNEL_CAP: usize = 8192;
    let (audio_tx, audio_rx) = mpsc::channel::<AudioChunk>(CHANNEL_CAP);
    let (trans_tx, mut trans_rx) = mpsc::channel::<TranscriptionChunk>(CHANNEL_CAP);

    let state = RecordingState::new();
    let live_stt = Arc::new(AtomicBool::new(true)); // starts LIVE (the owner's scenario)

    let pipeline = AudioPipeline::new(
        audio_rx,
        trans_tx,
        state,
        100,
        CAPTURE_RATE,
        "Test Microphone".to_string(),
        InputDeviceKind::Wired,
        "Test System".to_string(),
        InputDeviceKind::Wired,
        live_stt.clone(),
    )
    .expect("construct AudioPipeline");

    let handle = tokio::spawn(pipeline.run());
    let chunk_len = (CAPTURE_RATE as usize / 50).max(1);

    // Phase 1: live, MICROPHONE only.
    common::feed_device(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE, DeviceType::Microphone)
        .await;
    common::feed_device(&audio_tx, &pad_48k, chunk_len, CAPTURE_RATE, DeviceType::Microphone).await;
    let phase1 = common::drain_settled(&mut trans_rx).await;

    // Phase 2: deferred, MICROPHONE only (nothing new out; the detach flushes phase 1's
    // still-open tail, which is still mic audio).
    live_stt.store(false, Ordering::SeqCst);
    common::feed_device(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE, DeviceType::Microphone)
        .await;
    common::feed_device(&audio_tx, &pad_48k, chunk_len, CAPTURE_RATE, DeviceType::Microphone).await;
    let phase2_tail = common::drain_settled(&mut trans_rx).await;

    // Phase 3: live again, SYSTEM only — a channel the resumed stage has never seen.
    live_stt.store(true, Ordering::SeqCst);
    common::feed_device(&audio_tx, &speech_48k, chunk_len, CAPTURE_RATE, DeviceType::System).await;
    let mut phase3 = common::drain_settled(&mut trans_rx).await;

    drop(audio_tx);
    handle
        .await
        .expect("pipeline task join")
        .expect("pipeline run ok");
    // The stop-time flush is where phase 3's still-open utterance lands.
    while let Ok(chunk) = trans_rx.try_recv() {
        phase3.push(chunk);
    }

    // Pre-toggle sanity: mic-only speech is attributed to the mic.
    let pre_toggle: Vec<&TranscriptionChunk> = phase1.iter().chain(phase2_tail.iter()).collect();
    assert!(
        !pre_toggle.is_empty(),
        "the live phase (plus its detach flush) must produce chunks to attribute"
    );
    assert!(
        pre_toggle
            .iter()
            .any(|c| c.channel == Some(ChannelTag::Microphone)),
        "mic-only speech before the toggle produced no Microphone-tagged chunk: {:?}",
        pre_toggle.iter().map(|c| c.channel).collect::<Vec<_>>()
    );

    // THE REGRESSION SURFACE: the resumed tail.
    assert!(
        !phase3.is_empty(),
        "the re-attached stage must produce chunks again"
    );
    assert!(
        phase3
            .iter()
            .any(|c| c.channel == Some(ChannelTag::System)),
        "resumed-tail segments lost their SYSTEM attribution: {:?} — the channel-window \
         clock and the segment clock must share the session audio base (spec 0051 WS1)",
        phase3.iter().map(|c| c.channel).collect::<Vec<_>>()
    );
    assert!(
        !phase3
            .iter()
            .any(|c| c.channel == Some(ChannelTag::Microphone)),
        "system-only audio after the toggle was attributed to the microphone: {:?} — a \
         resumed segment must not inherit pre-gap channel spans (channel_windows must be \
         cleared on attach)",
        phase3.iter().map(|c| c.channel).collect::<Vec<_>>()
    );
}
