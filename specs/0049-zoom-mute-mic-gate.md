# 0049 — Zoom-mute detection → owner mic gate

- **Status:** Done — live-Zoom smoke passed; shipping
- **Owner agent(s):** audio-engineer + rust-core-engineer + frontend-engineer
- **Roadmap phase:** Capture fidelity (feedback-driven)

## Context / Problem
Zoom's mute button is a **software mute inside Zoom** — it stops Zoom transmitting your
audio to the meeting, but your macOS microphone stays open, so Vinyl keeps capturing and
transcribing whatever you say while muted (side conversations, thinking aloud). The owner
reported this directly. Spec 0047 fixed *remote* audio bleeding into the mic being labelled
"You"; it does **not** address the owner's own genuine speech while muted (that's mic-dominant
and correctly "You" — just unwanted).

Decision (owner, this session): detect Zoom's mute state automatically and, **while muted,
drop the owner microphone from the transcript** (system/remote audio keeps recording). Zoom
desktop app only. The owner accepted the cost of the macOS Accessibility route (see Approach).

## Goals
- Detect mute/unmute in the **Zoom desktop app** on macOS, near-real-time.
- While muted: owner mic contributes **nothing** to the live transcript, the offline
  diarization owner-turns, or `mic.wav`. System/remote audio is unaffected.
- Opt-in (off by default); clear UI state ("muted — mic paused") and a permission flow.

## Non-goals
- Google Meet / Microsoft Teams (browser/app, different + harder detection) — deferred.
- Perfect reliability across every Zoom version/localization (AX is inherently brittle; we
  fail *safe* = when we can't read mute state, behave as today and keep transcribing).
- Sample-accurate gating at the mute boundary — a poll-interval of the owner's audio may
  slip through right at a mute/unmute edge (acceptable; see Risks).

## Approach
Two independent halves; the split matters because their cost is wildly different.

**A. Mic gate (cheap, no new deps).** Mirror the existing `is_paused` pattern: an
`is_muted: AtomicBool` on `RecordingState`, read in `AudioCapture::process_audio_data`. When
set, drop owner-mic chunks before they enter the mixer. The mixer's existing
"no mic data → zero-pad" path (`extract_window`) then yields system-only transcription
windows and a silent `mic.wav` automatically — no downstream changes. Flag is shared/flipped
using the proven `live_toggle.rs` session-flag pattern.

**B. Zoom mute reading (the whole cost) via macOS Accessibility (AX).** No present crate
wraps the AX C API, so this needs a new dependency (or hand-rolled `ApplicationServices` FFI)
and the **Accessibility TCC permission** — which, unlike every current Vinyl permission, has
no Info.plist key and must be prompted via `AXIsProcessTrustedWithOptions` + a manual
System-Settings hand-off. Isolated behind a single `zoom/mute.rs` module (as spec 0008
isolated the CptHost detection), and explicitly opt-in. spec 0008 evaluated and *rejected*
this path as brittle/heavyweight; we revisit it deliberately with eyes open.

## Design
### Rust — mic gate
- `audio/recording_state.rs`: add `is_muted: AtomicBool` alongside `is_paused`; `is_muted()` /
  `set_muted(bool)`; construct `false`. Already `Arc`-shared into every `AudioCapture`.
- `audio/pipeline.rs` `process_audio_data` (after the `is_recording` early-return):
  `if self.device_type == DeviceType::Microphone && self.state.is_muted() { return; }`.
- Share/flip via a `live_toggle.rs`-style session flag so the mute poll task can set it.

### Rust — Zoom mute reader (`zoom/mute.rs`, new)
- `zoom_pid()` — reuse the `sysinfo` scan already in `zoom/monitor.rs` (the `cpthost`/`aomhost`
  helper PID).
- `read_mute_state(pid) -> Option<bool>` — `AXUIElementCreateApplication(pid)`, walk to the
  mute control, read its state. **Exact element/attribute is an open question** (see below) —
  isolate it here so a Zoom-UI change is a one-file fix.
- **Dependency decision (open):** `accessibility` + `accessibility-sys` crates (standard,
  safer) vs hand-rolled `#[link(name = "ApplicationServices", kind = "framework")]` extern "C"
  over the ~4 AX functions we need (zero new crate, reuses present `core-foundation`). Lean
  hand-rolled to avoid a dependency, pending a spike.
- Mute poll task (separate from the 3s meeting loop): poll ~500 ms **only while a Zoom meeting
  is active** (gate on the existing `zoom-meeting-detected`/`-ended` events), a light 2-poll
  debounce (own pure state machine, unit-testable like `DebounceMachine`), flip `is_muted`,
  emit `zoom-mute-changed` for the UI. Stops polling when no meeting / feature off.

### Rust — permission + settings
- `zoom/settings.rs`: add `zoom_mute_gate: bool` (opt-in, default `false`).
- Permission: on enabling, call `AXIsProcessTrustedWithOptions({prompt:true})`; if not trusted,
  surface guidance + open `x-apple.systempreferences:...Accessibility` (reuse
  `audio/permissions.rs` pattern). Re-check trust before each poll session.

### Frontend
- Settings toggle "Pause my mic when muted in Zoom (Accessibility)" with the permission
  hand-off. A recording indicator "🔇 Muted in Zoom — mic paused" driven by `zoom-mute-changed`.

## Tasks
1. [x] Mic gate: `is_muted` on `RecordingState` + drop mic chunks in `send_audio_chunk`
   (mirrors the `is_paused` gate; the mixer then zero-pads the mic window → system-only
   transcript + silent `mic.wav`). TDD'd (`mute_gate_tests`). Committed; inert until a driver
   sets the flag. (Gated in `send_audio_chunk`, not `process_audio_data` — same effect, more
   testable, mirrors the existing pause gate.)
2. [~] AX spike: **`scripts/zoom-ax-probe.sh`** dumps Zoom's mute-related AX tree via System
   Events (same AX API, no dependency, no build). **Awaiting a live-Zoom run** (muted vs
   unmuted diff) to pin the stable element/attribute; then update this spec and pick the
   Task-3 dependency (crate vs hand-rolled FFI). Using System Events for the spike defers the
   AX-crate/FFI decision until we know exactly what to read.
3. [x] `zoom/mute.rs` reader (via the `accessibility`/`accessibility-sys`/`core-foundation`
   crates — chosen over hand-rolled FFI for correctness) + `zoom/mute_monitor.rs` poll task.
   Signal: the "Meeting" menu's self-mic item ("Mute audio"=unmuted / "Unmute audio"=muted).
   The poll uses the existing `is_recording()` and flips a process-wide gate.
4. [x] Permission flow (`api_zoom_mute_ax_trusted` prompt + `api_open_accessibility_settings`)
   + `zoom_mute_gate` setting (`serde(default)`) + get/set commands, registered.
5. [x] Frontend `ZoomMuteGateToggle` (opt-in + permission hand-off) + `ZoomMuteIndicator`
   ("🔇 Muted in Zoom" toast on `zoom-mute-changed`).
6. [x] **Manual smoke with real Zoom** — passed (owner mic gates on mute; system/remote audio
   unaffected; indicator shows).

**Architecture note (built):** the mic gate is a process-wide flag in `audio/mute_gate.rs`
(not a field on `RecordingState`) — the poll flips it via `is_recording()` without a handle to
the active recording, `send_audio_chunk` reads it, and `stop_recording` clears it. This kept
the change to new modules (size ratchet) and avoided threading a `RecordingState` handle out.

## Acceptance criteria
- With the feature on and Accessibility granted: muting in Zoom stops the owner mic appearing
  in the live transcript within ~1 s; unmuting resumes it; `mic.wav` is silent across muted
  spans; system/remote transcription is unaffected. Feature off or permission denied → today's
  behavior exactly (fail-safe).

## Risks / open questions
- **AX fragility:** the mute element/attribute (and any title strings) are localized and
  change across Zoom versions — Task 2 must find the most stable signal; isolate in one file;
  fail-safe to "keep transcribing" on any read failure.
- **Permission friction + dev signing:** AX grant is heavier than any current permission and,
  per CLAUDE.md, ad-hoc dev re-signing orphans TCC grants — dev testing needs the stable
  Apple-Development identity path.
- **Boundary latency:** up to one poll interval of owner audio can slip at a mute/unmute edge.
  Tighten poll vs CPU; accept a small edge.
- **Dependency:** prefer zero-new-crate FFI, but validate against the safer `accessibility`
  crate in the spike.

## Verification
Unit: mic-gate mixer behavior + pure debounce. Manual: real Zoom meeting, toggle mute, confirm
transcript/`mic.wav` behavior and the UI indicator. No automated AX test (needs live Zoom).
