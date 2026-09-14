# ADR-0001 — Fork meetily as the foundation

- **Status:** Accepted
- **Date:** 2026-06-23

## Context
We want a local-first macOS app that records, transcribes, summarizes, and organizes
meetings at granola.ai quality. Building from scratch means reimplementing the hardest part —
gapless macOS system+mic capture, mixing, VAD, and real-time GPU-accelerated transcription —
which is months of work. meetily (MIT) already does this at production quality, including a
Core Audio process-tap approach that needs **no BlackHole/virtual device**.

## Decision
Fork meetily and build our differentiators on top, rather than build greenfield.

## Consequences
- We inherit a ~44k-LOC Rust/Tauri + Next.js codebase and its conventions (and some tech
  debt: `*_old.rs`, an unfinished `audio_v2/` refactor).
- We commit to the Rust + Tauri + Next.js stack.
- We keep meetily's audio/STT/persistence/shell and rebuild the note-enhancement UX,
  diarization, and search/organization.
- We must avoid pulling meetily PRO-licensed code; we stay on the MIT community edition.

## Alternatives considered
- **Greenfield:** maximal control, but the audio engine alone would delay any usable product
  by months. Rejected.
- **Stay close to upstream (overlay):** most mergeable, but constrains the re-architecture we
  need. Rejected in favor of a hard fork (see ADR-0003).
