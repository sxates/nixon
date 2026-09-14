# 0023 — Test strategy & recording-lifecycle regression harness

- **Status:** In progress
- **Owner agent(s):** rust-core-engineer + frontend-engineer (+ audio-engineer for the audio harness)
- **Roadmap phase:** Post-1.0 hardening (cross-cutting; unblocks every other spec)

> **Landed so far:** L2 WS6.7 regression (`tests/db_lifecycle.rs::second_session_does_not_merge_into_populated_meeting`)
> + the WS6.1 audio-guard unit tests (`api.rs::discard_guard_tests`); **L3 stood up** —
> Vitest + RTL + jsdom (`frontend/vitest.config.ts`, `vitest.setup.ts`), a `pnpm test`
> script, and first tests (`src/lib/__tests__/{day-agenda,calendar}.test.ts`,
> `src/services/__tests__/storageService.test.ts`) + **hook-level lifecycle tests for all
> three seams** (`useRecordingStart`, `useRecordingStop`, `TranscriptContext`) — **26 frontend
> tests green**. **CI wired**: `.github/workflows/ci-checks.yml` gates PRs on lint+test
> (frontend) and clippy+lifecycle tests (Rust/macOS); DoD updated. Remaining: L4 E2E
> (tauri-driver), L5 manual checklist, and the L2 discard-guard repo cover.

## Context / Problem

1.0 testing surfaced a steady stream of bugs, concentrated in the **recording → save → reopen
lifecycle**, and several are **data loss / corruption**:
- `specs/0019` WS6.1 — a finished meeting was auto-deleted (zero-transcripts false signal).
- `specs/0019` WS6.7 — two consecutive recordings merged into one meeting; the other's transcript
  was lost (stale `currentMeeting.id` + blind backend append/overwrite).

These share a root pattern: **frontend session state leaks across recordings**, and the backend
**trusts the id it's handed** without invariant checks. They are cheap to catch with tests and
expensive to catch by hand — yet today there is **no automated coverage of the lifecycle and no
frontend tests at all**. We keep finding these in production.

**What exists today:**
- Rust integration tests in `frontend/src-tauri/tests/`: `db_lifecycle.rs` (already drives the
  real `create_meeting → save_transcripts_for_meeting → get_meetings_enriched → delete_meeting`
  repo path over a temp-file SQLite DB via `DatabaseManager::new`, **no Tauri runtime, no audio**),
  plus `transcription_engine.rs`, `vad_filter.rs`, `pipeline_integration.rs`,
  `diarization*.rs`; shared helpers in `tests/common/`; `tempfile` is a dev-dep. (`specs/0009`.)
- A handful of in-crate `#[cfg(test)]` unit modules (e.g. `api.rs` gist tests; the new
  `discard_guard_tests` from WS6.1).
- CI workflows under `.github/workflows/` (`pr-main-check.yml`, `build-test.yml`, per-OS builds).
- The Definition of Done in `/CLAUDE.md` (cargo check/clippy, `pnpm lint`, app launches, manual
  smoke). **`pnpm` has no `test` script; no Vitest/Jest/RTL/WebDriver; no E2E.**

**The gap:** the layer where the lifecycle bugs actually live — the React hooks driving
`currentMeeting`/`existingMeetingId`/session state (`useRecordingStart.ts`, `useRecordingStop.ts`,
`TranscriptContext.tsx`) — has no tests, and the DB-lifecycle test asserts only the happy single
session, not the **multi-session invariants** these bugs violate.

## Goals

- A layered, mostly-fast test suite that runs in CI on every PR and is part of the DoD.
- Turn each known lifecycle bug into a **named invariant** with a regression test that fails on
  the old behavior and passes on the fix.
- Stand up the **missing frontend test layer** (Vitest + React Testing Library) and cover the
  recording-lifecycle hooks with mocked Tauri IPC.
- Extend the existing Rust DB-lifecycle harness with **multi-session** scenarios.
- A deterministic, audio-free path to exercise record→save→reopen end-to-end where possible, and
  a short **manual smoke checklist** for the irreducible hardware-capture parts.

## Non-goals

- 100% coverage or testing the Core Audio tap / Whisper/Parakeet model quality (kept as manual
  smoke + the existing engine fixtures; model accuracy is out of scope).
- A full cross-platform E2E grid this round (macOS is the supported target per `/CLAUDE.md`).
- Replacing manual verification of hardware mic/screen-recording capture (cannot be automated in
  CI; stays a checklist item).

## Approach — the test pyramid (fast → slow)

| Layer | Where | Runtime | Catches |
|---|---|---|---|
| **L1 Rust unit** | in-crate `#[cfg(test)]` | ms | pure logic (folder/audio guards, gist, parsers) |
| **L2 Rust integration** | `src-tauri/tests/*.rs` over temp SQLite | sub-sec | DB/repo lifecycle **invariants**, diarization persistence |
| **L3 Frontend unit/component** | **NEW** Vitest + RTL, mocked `invoke`/`listen` | sub-sec | hook state machines: `currentMeeting`/`existingMeetingId`, buffer clearing, pending-key |
| **L4 E2E smoke** | **NEW** tauri-driver/WebDriver + the `specs/0009` audio harness | seconds | record→stop→reopen wired through real IPC |
| **L5 Manual smoke** | checklist (`docs/`) | minutes | Core Audio tap, screen-recording perms, real Zoom |

Most regressions (including WS6.1 and WS6.7) are catchable at **L2 + L3** — fast, deterministic,
CI-friendly — so invest there first; L4 is a thin happy-path safety net; L5 stays minimal.

## Design

### Lifecycle invariants (the backbone — each becomes assertions)

These properties must hold across the record→save→reopen flow. Each maps to a test and to a known
bug:

1. **One recording ⇒ exactly one new meeting row.** Two consecutive recordings produce two
   distinct `meeting_id`s. *(WS6.7)*
2. **Transcripts never cross meetings.** Every transcript row's `meeting_id` belongs to the
   session that produced it; no meeting contains segments from two recordings. *(WS6.7)*
3. **`folder_path` is stable and 1:1.** A meeting's `folder_path` is set once to its own
   recording folder and never overwritten by a different session; two meetings never share a
   folder. *(WS6.7)*
4. **No durable content is auto-deleted.** A calendar-linked meeting, a meeting with persisted
   transcripts, or one with on-disk audio is never removed by the abandoned-recording cleanup.
   *(WS6.1 — partially covered by `discard_guard_tests`; extend to the repo level.)*
5. **No orphaned audio / no orphaned row.** After a normal stop, the meeting row references its
   folder and the folder exists; after a discard, neither remains. *(WS6.1)*
6. **Live buffer is per-session.** Starting a new recording clears the prior session's transcript
   buffer; segments from A cannot be saved into B. *(`TranscriptContext`.)*
7. **Re-save is idempotent.** Saving the same session twice does not duplicate/interleave rows.
   *(append-vs-replace, `transcript.rs`.)*

### L2 — extend the Rust DB-lifecycle harness

In `frontend/src-tauri/tests/` (new `recording_lifecycle.rs`, or extend `db_lifecycle.rs`),
reusing `fresh_db()`:
- `two_sessions_two_meetings_disjoint_transcripts` — `create_meeting` twice, save disjoint
  segments to each; assert two ids, transcripts partitioned by id, distinct `folder_path`s
  (invariants 1–3). **This is the WS6.7 regression.**
- `save_under_existing_id_does_not_silently_merge` — pin the desired post-fix contract for
  `save_transcripts_for_meeting` when a meeting already has segments from another session
  (replace, or reject — decide in WS6.7); fails on today's blind append (`transcript.rs:147-148`)
  and folder overwrite (`:124-132`).
- `discard_only_when_no_durable_content` — repo/command-level cover for invariants 4–5
  (calendar-linked, has-transcripts, has-audio → kept).
- `resave_is_idempotent` — invariant 7.

### L3 — stand up the frontend test layer (NEW)

- Add **Vitest + @testing-library/react** + a `test` script to `frontend/package.json`; mock
  `@tauri-apps/api` `invoke`/`listen` (and `plugin-notification`) with a small fake IPC.
- Cover the lifecycle hooks with the invariants above:
  - `useRecordingStart.ts` — `meetingCreatedRef` resets between sessions; `createMeetingForRecording`
    mints a fresh id each session; failed `api_create_meeting` does **not** reuse the prior id;
    `consumePendingJoinMeeting`/`PENDING_JOIN_MEETING_KEY` is cleared and not adopted by an
    unrelated next recording.
  - `useRecordingStop.ts` — after a successful save, `currentMeeting` is reset (or
    `existingMeetingId` is captured at start) so the **next** stop cannot target the prior id;
    the abandoned path only deletes when `api_recording_is_safe_to_discard` returns true.
  - `TranscriptContext.tsx` — buffer/segments cleared on new `currentMeetingId` (invariant 6).
- These tests assert the exact state transitions that WS6.7 and WS6.1 hinge on, with no backend.

### L4 — E2E smoke (thin)

- Add `tauri-driver` + a WebDriver client (e.g. WebdriverIO) for one happy-path script:
  start recording (feeding the deterministic fixture audio from `specs/0009` rather than a live
  mic) → stop → reopen the meeting → assert the transcript renders and belongs only to that
  meeting. Gated behind a CI job that can run a headless Tauri build; keep it small.

### L5 — manual smoke checklist

- A `docs/MANUAL_SMOKE.md` checklist for the parts CI can't cover (Core Audio system-audio tap,
  screen-recording permission, real Zoom join/auto-detect/auto-stop), aligned with `/CLAUDE.md`
  DoD #4. Run before a release (`release.sh`).

### CI gating

- Extend `.github/workflows/pr-main-check.yml` (or `build-test.yml`) to run on PRs:
  `cargo clippy` + `cargo test --features metal` (the macOS runner) and, in `frontend`,
  `pnpm lint` + `pnpm test` (L3). Fail the PR on any failure. L4 in a separate, possibly
  non-blocking job initially.
- Update `/CLAUDE.md` Definition of Done to require L1–L3 green (it already lists clippy/lint/smoke).

## Tasks
1. [x] L2: WS6.7 regression (invariants 1–3) — `tests/db_lifecycle.rs::second_session_does_not_merge_into_populated_meeting`. *(invariant 7 folded in: a re-save no longer duplicates — it routes to a new row.)*
2. [ ] L2: repo/command cover for the discard guard, invariants 4–5 (the pure audio guard is unit-tested in `api.rs::discard_guard_tests`; the calendar-link/transcript-count parts of `api_recording_is_safe_to_discard` need extraction to be integration-testable without an `AppHandle`).
3. [x] L3: Vitest + RTL + jsdom + `pnpm test` + Tauri IPC mock pattern (`vi.hoisted` + `vi.mock('@tauri-apps/api/core')`). Build approval for `esbuild` recorded in `frontend/pnpm-workspace.yaml`.
4. [x] L3: hook tests for all three lifecycle seams (contexts mocked at the module boundary):
   - `useRecordingStart` (`__tests__/useRecordingStart.test.ts`): fresh-row mint, Join & Record adoption + pending-key consume, per-session idempotency, model-not-ready block (WS6.7 START side).
   - `useRecordingStop` (`__tests__/useRecordingStop.test.ts`, fake timers): abandoned-cleanup deletes only when `api_recording_is_safe_to_discard` is true; otherwise PRESERVES the meeting via save (WS6.1).
   - `TranscriptContext` (`__tests__/TranscriptContext.test.tsx`): each `recording-started` mints a distinct `currentMeetingId` (the per-session buffer-reset trigger — invariant 6).
5. [ ] L4: `tauri-driver` happy-path record→reopen smoke using the `specs/0009` fixture (frontend + audio).
6. [x] L5: `docs/MANUAL_SMOKE.md` release checklist — covers the hardware/Zoom paths CI can't, plus explicit data-integrity scenarios for WS6.1/WS6.3/WS6.4/WS6.5/WS6.7 and the WS8 release check.
7. [x] CI: `.github/workflows/ci-checks.yml` runs frontend lint+test (ubuntu) and Rust clippy+lifecycle tests (macOS/Metal, builds the sidecar) on PRs/pushes; DoD in `/CLAUDE.md` updated to require `cargo test` + `pnpm test`.

## How to run (current)
- Rust: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal`
  (lifecycle: `--test db_lifecycle`).
- Frontend: `cd frontend && pnpm test` (one-shot) or `pnpm test:watch`.
  - First run on a clean machine needs the `esbuild` build script approved — it's in
    `frontend/pnpm-workspace.yaml` under `allowBuilds`/`onlyBuiltDependencies`; if pnpm reports
    `ERR_PNPM_IGNORED_BUILDS`, run `pnpm rebuild esbuild`.
- Test file conventions: co-locate under `__tests__/` as `*.test.ts(x)`; import test globals
  from `vitest`; mock Tauri IPC with `const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
  vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));`.

## Acceptance criteria
- `cargo test --features metal` runs the lifecycle suite; the multi-session test **fails on the
  pre-WS6.7 code and passes after the fix** (proves it's a real regression guard).
- `pnpm test` exists and runs L3; the `useRecordingStop` success-path-reset test fails on the
  current stale-`currentMeeting` behavior and passes after the WS6.7 frontend fix.
- CI blocks a PR that breaks any L1–L3 test or lint.
- Each invariant in the table above has at least one owning test.
- DoD per `/CLAUDE.md` updated and green.

## Risks / open questions
- L4 (`tauri-driver`) on macOS CI can be flaky and slow — start non-blocking; the L2/L3 layers are
  the real safety net.
- Tauri IPC mocking surface: standardize one fake-`invoke` helper so hook tests stay maintainable.
- Some invariants (e.g. live-buffer clearing) straddle hook + context; decide test seams to avoid
  brittle full-render tests — prefer testing the hooks' reducers/refs directly where possible.
- WS6.7's backend contract (replace vs. reject on cross-session save) must be decided before
  writing `save_under_existing_id_does_not_silently_merge` so the test pins the intended behavior.

## Verification
- `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal` (L1+L2).
- `cd frontend && pnpm test` (L3), `pnpm lint`.
- L4: the WebDriver smoke job locally/CI.
- L5: walk `docs/MANUAL_SMOKE.md` before release.
- Bug-driven proof: check out the pre-fix commit, confirm the WS6.7 L2 + L3 tests **fail**; apply
  the fix, confirm they pass.
