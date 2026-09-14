# Spec 0030 — 0028 Deferred Follow-ups + Housekeeping

**Status:** ✅ Implemented (2026-07-01) — all gates green (known-flaky `vad_filter` adversarial
test excluded, fails on pristine HEAD). Keychain flow needs the manual signed-build
verification pass listed in ADR-0009.
**Branch:** `chore/0030-deferred-followups`
**Parent:** `specs/0028-robustness-hardening.md` → "Deferred / follow-ups"

## Why now

v1.3.0 shipped with a real Developer ID signature + notarization (ADR-0008), which was the
stated blocker for the Keychain migration ("needs signed-build entitlement validation").
This spec batches the deferred 0028 items that are now unblocked, plus documentation
housekeeping identified in the 2026-07-01 backlog review.

## Workstreams

### WS1 — API keys → macOS Keychain (0028:422-424, `setting.rs:27` TODO)
Move LLM/transcription API keys out of cleartext SQLite into the macOS Keychain.

- Keychain is the primary store; service name derives from the bundle identifier
  (via `app_paths`) so dev (`ai.vinyl.app.debug`) and prod (`ai.vinyl.app`) stay isolated.
- One-time migration on startup: existing DB keys are written to the Keychain; on success
  the DB value is replaced with a sentinel, on failure the DB value is left untouched
  (graceful fallback — never break summarization).
- Reads prefer Keychain, fall back to DB when the value is not the sentinel. If the
  Keychain item is inaccessible (e.g. dev ad-hoc re-sign) and the DB holds only the
  sentinel, the provider reports *not configured* rather than erroring.
- Covers both storage shapes: the settings table columns AND the nested JSON blob
  (model settings), across anthropic/openai/groq/openrouter + transcription providers.
- ADR-0009 documents the design + fallback semantics.

### WS2 — API-key getter returns "is-configured", not cleartext (0028:425-426)
The Tauri getter commands stop returning raw keys to the frontend; they return a
configured/masked shape instead. The ~4 frontend settings read-sites switch to the new
shape (show "configured •••• last4" style state; setting a key stays write-only).

### WS3 — Pin real ffmpeg SHA-256 digests (0028:427-428, `build/ffmpeg.rs:343`)
Fill the per-target digest constants with real values computed from the pinned download
URLs and make a mismatch fail the build instead of warning. If any URL is not
version-pinned, pin it first.

### WS4 — Surface backpressure events + partial-summary status in the UI (0028:429-431)
Wire `transcription-chunk-skipped` / `transcription-chunks-skipped` /
`transcription-chunks-dropped` into a user-visible toast/banner (same pattern as
`transcription-falling-behind`), surface the `summary_status` partial-summary field in the
summary UI, and resolve the payload-type `TODO(0028)` in `TranscriptContext.tsx`.

### WS5 — Documentation housekeeping
- CLAUDE.md: drop the stale `*_old.rs` / `audio_v2` tech-debt note (cleanup shipped in 0.2.0).
- ROADMAP.md: check off items shipped since it was written (notes-only meetings, P2
  speaker rename/merge, etc.), with pointers to the shipping spec/version.
- specs/BACKLOG.md: mark the three 2026-06-26 items resolved (all shipped) with pointers.
- ADR-0004: document the dev-build notification-icon caching gotcha + `lsregister` reset
  (0029 WS6.3 follow-up).

### WS6 — Review deliverables (no code)
- Roadmap review: `docs/reviews/2026-07-01-roadmap-review.md` — critique + improvement
  recommendations for ROADMAP.md and the Phase 4/5 plans.
- Fork-deprecation audit: `docs/audits/2026-07-01-fork-deprecation-audit.md` — inherited
  meetily surface area that is dead, unused, or simplifiable now that Vinyl has diverged.

## Out of scope
Next.js 14→15 and reqwest 0.12 bumps (still not to be attempted unattended); mixer
timestamp alignment + cpal thread refactor (need a real-audio soak test); WS1.2 Zoom-hang
and WS6.3 icon verification from 0029 (need the user's machine).

## Acceptance
1. `cargo check` / `clippy -D warnings` / `cargo test` clean (metal), `pnpm lint` / `pnpm test` clean.
2. Keys never cross IPC in cleartext; a fresh install and an upgraded install both keep
   working summaries for every provider (Keychain manual verification noted for a signed build).
3. ffmpeg build fails closed on digest mismatch.
4. Backpressure/skip/drop conditions and partial summaries are user-visible.
