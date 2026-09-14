# ADR-0005 — Speaker-diarization engine: sherpa-onnx (via `sherpa-rs`), offline, per-channel

- **Status:** Accepted
- **Date:** 2026-06-24
- **Related:** `specs/0010-speaker-diarization.md`, ADR-0001 (fork), `specs/0008` (calendar attendees)

## Context
Speaker diarization ("who said what") is Vinyl's flagship differentiator (meetily has none; it's
paywalled in their PRO). We need an **on-device** engine that fits the privacy thesis (no cloud), a
Rust/Tauri app (no Python runtime — `CLAUDE.md` forbids it), and our existing inference stack
(`ort = 2.0.0-rc.10` / ONNX Runtime, already shipped for the Parakeet STT engine).

## Decision
1. **Engine: sherpa-onnx via the `sherpa-rs` crate (0.6.8).** It exposes a first-class **offline**
   diarization pipeline (pyannote segmentation-3.0 → speaker embedding → clustering) with a Rust API,
   runs on ONNX Runtime (no second ML framework, no Python), and ships small, permissively-licensed
   models (segmentation MIT ~6.6 MB; 3D-Speaker embedding Apache-2.0 ~28 MB) downloaded on demand like
   our Whisper/Parakeet models. A feasibility spike compiled, linked, and ran `ort 2.0.0-rc.10` **and**
   `sherpa-rs 0.6.8` in one process with no symbol conflict (they link in different modes: `ort`
   statically links ORT 1.22; `sherpa-rs-sys` dynamically links ORT 1.17.1 dylibs).
2. **Offline (post-meeting) first.** Diarize the finished recording, where the whole conversation is
   available for stable clustering and it never competes with real-time STT. Live/streaming is deferred
   to P3.
3. **Diarize the SEPARATED streams, not the mix.** The microphone channel is the **local user** (`You`,
   no clustering needed); diarization runs only on the **system channel** (the unknown remote speakers).
   This improves accuracy and halves the audio to cluster. Requires retaining per-channel audio
   (`system.wav` + `mic.wav`, 16 kHz mono).
4. **A `Diarizer` trait** abstracts the engine so **speakrs** (pure-Rust, CoreML, higher accuracy but
   pulls in BLAS) remains a low-cost P3 swap if sherpa accuracy/latency disappoints.

## Consequences
- **Packaging work (the main cost):** sherpa's two dylibs (`libonnxruntime.1.17.1.dylib`,
  `libsherpa-onnx-c-api.dylib`) must be bundled in the `.app` with a correct `@executable_path` rpath,
  mirroring the `ffmpeg`/`llama-helper` sidecar handling. Must be verified with a real bundled-app smoke
  (`./build-gpu.sh` + launch), not just `cargo build`.
- **Two ONNX Runtimes in one process** (1.22 static + 1.17.1 dynamic) is unusual but observed-working at
  init; a P1 smoke test must run a Parakeet session and a diarization `compute()` in the same process.
- Reuses the reserved `speakers.embedding` BLOB + (new) `email` key for the P2 calendar-attendee
  association and P3 cross-meeting "People" identity.

## Alternatives rejected
- **speakrs** (P1): extra native-dep surface (BLAS); kept as the P3 upgrade path behind the trait.
- **pyannote via Python sidecar:** reintroduces a forbidden Python runtime.
- **Cloud diarization (Deepgram/AssemblyAI):** violates the local-first privacy thesis.
- **whisper-only mic-vs-system labeling** (the meetily approach): can't separate multiple remote
  speakers — used only as the mic-side shortcut and a graceful fallback.
