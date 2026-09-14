# 0028 — Robustness Hardening (Audit Remediation)

- **Status:** Draft
- **Owner agent(s):** spec-architect (coordination) → audio-engineer, rust-core-engineer, frontend-engineer, llm-pipeline-engineer (execution)
- **Roadmap phase:** Phase 2 (hardening)

## Context / Problem

A six-specialist audit (audio, rust-core, frontend, llm, security, CI) swept the Tauri/Rust
core and Next.js frontend and surfaced a cluster of correctness, privacy, real-time-safety,
and supply-chain defects. Several are **silent data-loss** paths: a meeting can be discarded
on a transcription timeout, the system-audio tap can self-terminate on a transient stall, and
transcription/summary chunks can be dropped without ever telling the user. Others are
**privacy regressions** that directly contradict the product's core promise ("meeting audio,
transcripts, and notes never leave the machine") — transcript text written to
`~/Library/Logs`, LLM API keys stored in plaintext SQLite, and whole-filesystem Tauri fs
grants. The remainder are real-time-unsafe locks in the audio callback, summary truncation,
frontend UX/accessibility gaps, and CI/supply-chain holes.

This spec is the **umbrella remediation plan** for that audit. It does not introduce new
product features; it is an ordered, checkable task breakdown so implementation agents can
execute the fixes with disjoint file ownership. Every task cites a real `file:line` (verified
against the tree at spec-authoring time) and a concrete fix.

All Rust paths are under `frontend/src-tauri/`; all frontend paths under `frontend/src/`.
Line numbers are anchors from the audit — treat them as "at or near"; the surrounding
function is named so the target is unambiguous if the file has drifted.

## Goals

- Eliminate every **silent data-loss** path (Critical tier): no meeting, transcript chunk, or
  summary chunk is discarded without either persisting what exists or surfacing a
  user-visible warning + retry.
- Close the **privacy regressions**: no transcript/summary content in release logs; API keys
  out of plaintext DB; no arbitrary-FS read/write from the webview; Rust-enforced analytics
  consent.
- Make the audio real-time path **RT-safe** and leak-free (no unwrap-on-poison locks in the
  IO-proc, no leaked taps, no `panic!`/`static mut` on the hot path).
- Fix summary **truncation / context-sizing / determinism** so long meetings summarize fully.
- Raise **frontend robustness & a11y** (poll lifecycle, autosave flush, toast over `alert()`,
  design tokens, tabpanel semantics).
- Harden **CI & supply chain** (clippy `-D warnings`, frozen lockfiles, pinned toolchains,
  checksummed downloads, dependency audits, de-Meetily'd release).

## Non-goals

- No new product features (diarization improvements, note-enhancement, search, etc. stay in
  their own specs).
- No migration of the archived Python `backend/` — it stays legacy reference.
- No rewrite of the meetily audio/mixing/VAD architecture beyond the targeted fixes here.
- The `audio/` → `audio_v2/` refactor is **not** in scope; do not build on `audio_v2`.
- Not every `any` at the IPC boundary must be typed in this spec — only the load-bearing
  boundary types (tracked as one Medium task, not 68 individual ones).

## Approach

Fix in **severity order** (Critical → High → Medium → Low), and within each tier group tasks
by **owner** so the four execution agents touch **disjoint files** and can run in parallel:

- **[audio]** — `audio/**` (capture, pipeline, vad, transcription/worker, stream,
  recording_manager), `whisper_engine/**`, `parakeet_engine/**`, `diarization/**`.
- **[llm]** — `summary/**` only.
- **[rust-core]** — `lib.rs`, `api/api.rs`, `database/**`, `analytics/**`,
  `tauri.conf.json`, settings.
- **[frontend]** — `frontend/src/**`.
- **[ci]** — `.github/workflows/**`, `Cargo.toml`, `package.json`, toolchain-pin files,
  `build/ffmpeg.rs`, `CONTRIBUTING.md`.

The one genuine cross-owner dependency — the **Critical dropped-chunk root cause** — is
called out explicitly: [audio] owns the bounded-queue + drop-policy plumbing
(`recording_manager.rs`, `pipeline.rs`, `worker.rs`) and emits a "falling behind" event;
[llm] owns the summary-chunk accounting (`processor.rs`). They coordinate on the shared
"processed vs skipped" contract but edit separate files.

*Why this shape:* the audit's own tiers map cleanly to blast radius, and disjoint file
ownership lets the four agents land small reviewable diffs without merge contention. An
alternative (one agent per tier) was rejected because it would serialize the work and force a
single agent across all subsystems.

## Design

### Shared "processed vs skipped" contract (Critical #3)

Transcription and summarization must stop conflating **dropped** with **completed**. Introduce
an explicit chunk-outcome enum on each pipeline (`Processed | Skipped { reason } | Failed
{ reason, retryable }`) and:

- **[audio]** replace the three **unbounded** `mpsc` channels with **bounded** queues
  (`tokio::sync::mpsc::channel(N)`) at `recording_manager.rs:77`, `pipeline.rs:1115`,
  `worker.rs:76`. On backpressure, apply an **explicit drop policy** (drop-oldest with a
  counter) rather than unbounded growth, and emit a Tauri **`transcription-falling-behind`**
  event so the frontend can warn. This bounds memory and removes the multi-minute stop hang.
- **[llm]** stop counting failed/unloaded chunks as completed; carry `Skipped`/`Failed`
  through to the caller and retry retryable failures.

### Data model / storage

- **API-key storage (High, privacy):** move LLM provider keys out of the `settings` table
  (plaintext) into the **macOS Keychain** via the `keyring` crate; keep only a
  reference/`is-configured` flag in SQLite. Interim mitigation: `chmod 0600` the DB file. A
  forward-only migration reads any existing plaintext key, writes it to Keychain, and nulls
  the column.
- **DB pool (High):** rebuild the pool from `SqliteConnectOptions` with `busy_timeout`,
  `journal_mode(Wal)`, and `foreign_keys(true)` so the hand-rolled 7-table cascade delete is
  backed by real FK enforcement.

### Tauri IPC / security

- Path-guard `read_audio_file` / `save_transcript` by reusing the existing
  `canonicalize + starts_with(root)` guard from `api/api.rs:1063`.
- Delete `fs:read-all` / `fs:write-all` from `tauri.conf.json` capabilities.
- API-key getters expose only `is-configured`, never cleartext, to the webview.
- `open_external_url` allow-lists `http`/`https`/`mailto` schemes only.

### UI

- Frontend: `useRef(Map)` for summary-poll intervals (mount-only cleanup), autosave flush on
  unmount, a shared **toast** helper replacing `alert()`, design-token colors replacing
  hardcoded Tailwind palette classes, and proper `tabpanel`/roving-focus a11y.

---

## Tasks

Ownership tags: **[rust-core] [audio] [llm] [frontend] [ci]**. Check a box only when the
change lands *and* its slice of the Definition of Done is green.

### CRITICAL — silent data loss

- [ ] **[frontend]** `hooks/useRecordingStop.ts:251` — save branch is gated on
  `isCallApi && transcriptionComplete == true`, so a transcription **timeout**
  (`transcriptionComplete=false`) skips the entire DB-save branch and discards the meeting.
  Persist whatever transcripts exist on timeout and show a warning toast; only skip the save
  when there are genuinely zero transcript segments.
- [ ] **[audio]** `audio/capture/core_audio.rs:309-311` (in `process_audio_data`) — the tap
  sets `should_terminate` after `>10` consecutive ring-buffer overflows and then returns
  `Poll::Ready(None)` at `:360`, permanently killing system-audio capture on a transient
  stall. Change to **drop samples and keep running** on overflow; only terminate on a real
  device/permission loss.
- [ ] **[audio]** `audio/transcription/worker.rs:145-147` (and `:259`) — when the model is
  unloaded the chunk is counted as **completed** (`chunks_completed.fetch_add`) without being
  processed. Distinguish `Processed` vs `Skipped`/`Failed`, surface skipped chunks to the
  user, and retry.
- [ ] **[llm]** `summary/processor.rs:610-616` — a chunk `Err(e)` branch only logs
  `error!` and `continue`s, silently dropping that chunk from the final summary. Carry the
  failure through (do not drop), surface it, and retry retryable failures.
- [ ] **[audio]** *Root cause of the dropped-chunk class:* replace the three **unbounded**
  channels — `audio/recording_manager.rs:77`, `audio/pipeline.rs:1115`,
  `audio/transcription/worker.rs:76` (all `mpsc::unbounded_channel::<AudioChunk>()`) — with
  **bounded** `mpsc::channel(N)` + an explicit drop-oldest policy and a dropped-count metric.
  Emit a `transcription-falling-behind` Tauri event. This bounds memory and removes the
  multi-minute stop hang from a single worker draining an unbounded backlog.
- [ ] **[frontend]** consume the new `transcription-falling-behind` event and show a
  non-blocking warning (pairs with the [audio] bounded-queue task above).

### HIGH — Privacy

- [ ] **[audio]** `audio/transcription/worker.rs:174,182` — `info!` logs full transcript
  text (`text='{}'`) to `~/Library/Logs`. Demote to a `debug!` macro compiled out in release,
  or log only length + confidence.
- [ ] **[audio]** `whisper_engine/whisper_engine.rs:788,793` — `log::info!` logs cleaned
  transcription text to disk. Same fix: debug-only / length+confidence.
- [ ] **[rust-core]** `database/repositories/setting.rs:70-140` — LLM API keys are written
  (`save_api_key`) and read (`get_api_key`) as plaintext in the `settings` table (also the
  initial migration). Move keys to the macOS **Keychain** via the `keyring` crate, keep only a
  reference/`is-configured` flag in the DB, add a forward-only migration, and `chmod 0600` the
  DB file as interim mitigation.
- [ ] **[rust-core]** `lib.rs:229` `read_audio_file` (`std::fs::read(&file_path)`) and
  `lib.rs:236` `save_transcript` (`create_dir_all` + `fs::write`, RCE-capable) accept an
  arbitrary path from the webview. Add the `canonicalize + starts_with(root)` guard reused
  from `api/api.rs:1063`.
- [ ] **[rust-core]** `tauri.conf.json:48-49` — capabilities grant `fs:read-all` and
  `fs:write-all` (whole filesystem). Delete both permission entries.
- [ ] **[rust-core]** `analytics/commands.rs:14` — `init_analytics` hardcodes
  `enabled: true`, so consent is only frontend-enforced. Enforce the persisted consent flag
  Rust-side before analytics initializes.

### HIGH — LLM (summary quality)

- [ ] **[llm]** `summary/llm_client.rs:248` — Claude request hardcodes `max_tokens: 2048`,
  truncating long summaries. Raise to 4–8k and make it configurable per model.
- [ ] **[llm]** `summary/service.rs:504-505` — Groq/CustomOpenAI (and other cloud providers)
  get a `100000` "effectively unlimited" context threshold, and `summary/processor.rs:336-339`
  (`use_single_pass`) forces single-pass for any non-Ollama/non-BuiltInAI provider. Size the
  threshold to each model's **real** context window (or let those providers chunk).
- [ ] **[llm]** `summary/llm_client.rs:222-226` — temperature/top_p/max_tokens are only
  forwarded when `provider == CustomOpenAI`; all other providers use the provider default.
  Pass a low temperature (~0.1) on all summary/translation passes.

### HIGH — Audio real-time safety

- [ ] **[audio]** `audio/capture/core_audio.rs:319` — `ctx.waker_state.lock().unwrap()` runs
  **inside the IO-proc real-time callback**; it is RT-unsafe and aborts the process on poison.
  Replace with an `AtomicWaker` (lock-free). (Note: the `:366` lock is on the async
  `poll_next` side, not the RT path — fix `:319` first.)
- [ ] **[audio]** `audio/capture/system.rs:55-86` — the system-audio forwarding task holds
  the `CoreAudioStream` open and can leak/keep the tap alive when the stream is silent at
  stop. Use `select!` on the stop signal + stream (or an abort handle) so the tap is dropped
  deterministically on stop.
- [ ] **[audio]** `audio/pipeline.rs:752` (in `ContinuousVadProcessor::new` match) —
  `panic!("VAD processor creation failed...")` on the start path. Return a `Result` and
  propagate the error instead of panicking.

### HIGH — Frontend

- [ ] **[frontend]** `components/Sidebar/SidebarProvider.tsx:326-330` — unmount cleanup does
  `activeSummaryPolls.forEach(clearInterval)`, so any change to the poll map kills **all**
  concurrent summary polls. Store intervals in a `useRef(Map)` and make the cleanup
  mount-only.
- [ ] **[frontend]** `app/_components/NotepadPanel.tsx:179-185` — unmount effect only
  `clearTimeout`s the pending autosave, dropping unsaved notes. Flush (save) the pending note
  on unmount before clearing the timer.
- [ ] **[rust-core]** `database/repositories/transcript.rs:319` (in `get_match_context`,
  fn @ `:306`) — `transcript[start_index..end_index]` is a **byte** slice and panics on
  non-ASCII / non-char-boundary UTF-8. Snap `start_index`/`end_index` to char boundaries.
- [ ] **[rust-core]** `lib.rs:595` — startup `.expect("Failed to initialize database")` on
  DB init crashes before the window exists. Surface the error to the user and keep the process
  alive.
- [ ] **[rust-core]** `database/manager.rs:33` — `SqlitePool::connect(...)` has no
  `busy_timeout`, no explicit WAL, and FKs disabled pool-wide (cascades are doc-only, a
  hand-rolled 7-table delete). Build the pool from `SqliteConnectOptions` with
  `busy_timeout` + `journal_mode(Wal)` + `foreign_keys(true)`.

### HIGH — Build / CI

- [ ] **[ci]** `.github/workflows/ci-checks.yml:95` — `cargo clippy --features metal` lacks
  `-D warnings`. Append `-- -D warnings` so lint regressions fail CI (matches Definition of
  Done #1).
- [ ] **[ci]** `.github/workflows/ci-checks.yml:96-98` — the transcription critical-path
  tests are model-gated/skip and `vad_filter` + `pipeline_integration` are excluded. Cache the
  model (or the `say`-generated fixtures) so these run in CI, and document what's gated and why.
- [ ] **[ci]** `.github/workflows/build.yml:83` pins pnpm `version: 8` against a
  `lockfileVersion: '9.0'` lockfile, and `build.yml:434` runs a bare `pnpm install` with no
  `--frozen-lockfile` (local uses pnpm 11). Unify on pnpm 11 + `--frozen-lockfile`.
- [ ] **[ci]** `.github/workflows/release.yml` (`:78` `Meetily v${version}` name, `:154`
  unwired `s3://meetily-updates/...` updater) and the `build-*.yml` variants are split-brain
  vs the canonical `release.sh`. Delete or rebrand them to Vinyl and wire (or remove) the S3
  updater; make `release.sh` the single source of truth.

### MEDIUM

- [ ] **[audio]** `audio/pipeline.rs:88` (`can_mix`) — mic/system buffers are mixed on length
  only, with no timestamp/offset alignment. Add timestamp-based alignment.
- [ ] **[audio]** `audio/vad.rs:185` (`resample_to_16k`) — crude linear-interp + basic
  low-pass resampler drops speech. Use a higher-quality resampler.
- [ ] **[audio]** `diarization/live.rs:240` (`snapshot_for_pass` clones the whole buffer;
  re-clustered every pass at `:374`/`:412`) — O(n²) full-buffer re-cluster, ~230 MB/hr. Move
  to incremental clustering / bounded buffer.
- [ ] **[audio]** `audio/pipeline.rs:52` — `static mut SAMPLE_COUNTER: u64` (mutated `:54`)
  is UB. Replace with an `AtomicU64`.
- [ ] **[audio]** `audio/pipeline.rs` — the "600 ms window" comments are wrong for the actual
  window size; correct the comments (or the constant) to match reality.
- [ ] **[audio]** `audio/stream.rs:29,38,363` (+ `recording_manager.rs:35`) — `unsafe impl
  Send` on cpal/CoreAudio-handle wrappers. Audit for genuine soundness; document or remove.
- [ ] **[audio]** `audio/stream.rs:342` — blocking `std::thread::sleep(50ms)` after
  `task_handle.abort()` in the async stop path. Replace with `tokio::time::sleep`/awaited
  join.
- [ ] **[audio]** remove `audio/recording_commands.rs.backup` (stray backup file).
- [ ] **[llm]** `summary/processor.rs:342-344` (`rough_token_count`, `chars * 0.35`) —
  under-counts CJK tokens. Use a script-aware estimate (~1 token/char for CJK).
- [ ] **[llm]** `summary/processor.rs:634-641` — the combine step joins all chunk summaries
  and calls `generate_summary` once, with no **recursive reduce** if the combined text exceeds
  context. Add a recursive reduce.
- [ ] **[llm]** add retries/backoff to summary LLM calls (none today).
- [ ] **[llm]** `summary/processor.rs:137-140` and `:575` — chunk-pass prompt interpolates
  raw `{chunk}` with no prompt-injection guard/delimiter hardening. Add a delimiter/injection
  guard.
- [ ] **[llm]** `summary/processor.rs:457-462` (`extract_meeting_name_from_markdown`) —
  accepts the `<Add Title here>` placeholder as an auto-rename title. Reject placeholder/empty
  headings.
- [ ] **[llm]** add summary **output evals** with fixtures (regression coverage for summary
  quality).
- [ ] **[llm]** `summary/llm_client.rs:272` — timeout message says "60 seconds" but the
  actual `REQUEST_TIMEOUT_DURATION` is 300s (`llm_client.rs:8`). Fix the message.
- [ ] **[llm]** `summary/llm_client.rs` — brittle code-fence / `<think>` stripping. Make the
  fence/think-tag stripping robust.
- [ ] **[frontend]** ~68 `any` / `as any` casts at the IPC boundary in `frontend/src` —
  introduce typed IPC wrappers for the load-bearing boundary types.
- [ ] **[frontend]** `components/Sidebar/SidebarProvider.tsx:336` — the context value is an
  inline object literal (not memoized), re-rendering all consumers. Wrap in `useMemo`.
- [ ] **[frontend]** replace native `alert()` with a toast in all sites:
  `contexts/TranscriptContext.tsx:376`, `components/RecordingControls.tsx:86,222,240`,
  `hooks/useRecordingStart.ts:314,408`, `components/AISummary/BlockNoteSummaryView.tsx:162`,
  `components/TranscriptRecovery/TranscriptRecovery.tsx:89,109`.
- [ ] **[frontend]** replace hardcoded Tailwind palette classes with design tokens:
  `app/meeting-details/page.tsx:381`, `components/MeetingDetails/SummaryPanel.tsx:397`,
  `components/AISummary/index.tsx:576+`, `app/_components/SettingsModal.tsx` (multiple).
- [ ] **[frontend]** `contexts/TranscriptContext.tsx:203`/`:395` — the transcript listener
  re-subscribes whenever `currentMeetingId` changes (effect dep). Decouple subscription from
  `currentMeetingId` so it does not churn the listener.
- [ ] **[frontend]** `app/meeting-details/page-content.tsx:284` — tab UI has `role="tablist"`
  / `role="tab"` but no `aria-controls`, no `role="tabpanel"`, no roving `tabIndex` /
  `onKeyDown`. Add tabpanel semantics + roving focus.
- [ ] **[frontend]** gate `console.log` of transcript/summary content behind a dev flag.
- [ ] **[rust-core]** remove dead `localhost:5167` backend commands: `api/api.rs:20`
  (`APP_SERVER_URL`), the `profile`/`license` commands (`api_get_profile` @ ~`:497` etc.), and
  `test_backend_connection` (~`:1529`).
- [ ] **[rust-core]** `lib.rs:71` `RECORDING_FLAG: AtomicBool` — recording state has two
  sources of truth (this atomic vs the audio module's state). Consolidate to one.
- [ ] **[rust-core]** `api/api.rs:685,578,711` return API keys / configs (with keys)
  cleartext to the webview. Expose only `is-configured`; never return the key value.
- [ ] **[rust-core]** `api/api.rs:1598` `open_external_url` runs `open` on any scheme.
  Allow-list `http`/`https`/`mailto` only.
- [ ] **[rust-core]** `whisper_engine/whisper_engine.rs:1121` and
  `parakeet_engine/parakeet_engine.rs:1074` — `cancel_download`/model-delete join an
  unsanitized `model_name` into a path (path traversal). Validate `model_name` against the
  known model registry before deleting.
- [ ] **[ci]** add `rust-toolchain.toml` (pin the Rust version).
- [ ] **[ci]** add `.nvmrc` and a `packageManager` field to `frontend/package.json` (pin
  Node + pnpm).
- [ ] **[ci]** `frontend/src-tauri/build/ffmpeg.rs:128-131` — ffmpeg is downloaded with no
  checksum (only a `-version` sanity check at `:333`). Pin and verify a SHA-256.
- [ ] **[ci]** sidecar auto-build gap — `llama-helper` is only built by
  `dev-gpu.sh`/`build-gpu.sh`. Add a `predev`/`prebuild` hook or a `build.rs` step so a bare
  `cargo build` / `pnpm tauri:dev` doesn't fail on the missing sidecar.
- [ ] **[ci]** `CONTRIBUTING.md:3,22` still references upstream Meetily (name + upstream
  remote URL). Rebrand to Vinyl.
- [ ] **[ci]** `.github/workflows/release.yml` (release build) doesn't run the CI gates. Run
  the Definition-of-Done gates in the release workflow.
- [ ] **[ci]** add `cargo audit` + `npm/pnpm audit` to CI.
- [ ] **[ci]** `frontend/src-tauri/Cargo.toml:57` (also `:128,:184`) pins `reqwest = "0.11"`.
  Bump to `0.12`.
- [ ] **[ci]** `frontend/package.json:79` `"next": "^14.2.25"` has client-side XSS advisories.
  Upgrade Next.js.

### LOW

- [ ] **[rust-core]** `reqwest` client is constructed per-call (declared 3×+, built per
  request). Move to a single shared `reqwest::Client`. (The `0.11→0.12` dep bump itself is the
  [ci] task above.)
- [ ] **[rust-core]** `tauri.conf.json:30` — stray `https://api.ollama.ai` in the CSP
  `connect-src`. Remove it (Ollama is local).
- [ ] **[ci]** scope the `ci-checks.yml` `push` trigger to PRs (avoid double-running on both
  push and PR).

---

## Acceptance criteria

Tie back to the Definition of Done in `/CLAUDE.md` (run `/check`):

1. **Gates green** — in `frontend/src-tauri`: `cargo check`, `cargo clippy` (now with
   `-D warnings`), and `cargo test` are clean; in `frontend`: `pnpm lint` and `pnpm test` are
   clean.
2. **App launches** via `./clean_run.sh` and the smoke path (record → live transcript →
   summary) still works.
3. **Critical tier verified by regression:** a simulated transcription timeout persists
   existing transcripts (no meeting loss); a simulated ring-buffer overflow keeps the tap
   alive; a simulated model-unload / failed summary chunk is reported as skipped/failed (not
   completed) and retried. Cover these with fixtures in `frontend/src-tauri/tests/` (aligned
   with `specs/0023`).
4. **Privacy:** a release build produces **no** transcript/summary text in `~/Library/Logs`;
   API keys are absent from the SQLite file (present in Keychain); the webview cannot read/write
   outside the app-data root; analytics stays off without persisted consent.
5. **CI:** clippy `-D warnings`, `--frozen-lockfile`, pinned toolchains, checksummed ffmpeg,
   and `cargo audit`/`pnpm audit` all run in `.github/workflows/`; the release workflow runs
   the gates; no "Meetily" branding remains in CI/release files.
6. Each checkbox above is checked only when its diff lands and its owner's slice of gates 1–2
   is green.

## Risks / open questions

- **Bounded-queue tuning (Critical #3):** too-small `N` drops audio under normal load;
  too-large re-introduces the memory/stop-hang problem. Needs a real-meeting soak test to pick
  `N` and confirm the drop-oldest policy is audibly acceptable.
- **Keychain migration:** first-launch migration must not lock out users if Keychain access is
  denied; keep a graceful fallback + clear error. Cross-check with the existing
  `data_migration.rs` first-launch flow.
- **RT-safety refactor (`AtomicWaker`)** touches the CoreAudio hot path — highest regression
  risk; requires the manual system-audio smoke test (bundled `Vinyl.app`, not the ad-hoc-signed
  dev binary — see ADR-0004 Screen-Recording gotcha) since it can't be unit-tested.
- **Next.js upgrade** may pull breaking changes into the Next 14 → 15 boundary; scope whether
  a patch bump clears the advisories or a major upgrade is required.
- **CI model caching** for the transcription tests: cache size vs. runner limits; may need a
  fixture-only path instead of the full model.
- Line numbers are audit-time anchors; a few had already drifted 1–2 lines (e.g. VAD panic is
  `:752` not `:750`, DB `.expect` is `:595`, `fs:*` grants `:48-49`) — implementers should
  match on the named function, not the raw line.

## Verification

- **Automated (Definition of Done layers 1–2):**
  `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo clippy
  --features metal -- -D warnings && cargo test --features metal --test transcription_engine
  --test vad_filter --test pipeline_integration --test db_lifecycle`; then
  `cd frontend && pnpm lint && pnpm test`.
- **New regression fixtures** in `frontend/src-tauri/tests/` for the Critical paths (timeout
  persistence, overflow survival, chunk-outcome accounting) — extend the `specs/0023` harness.
- **Privacy checks:** grep a release-build log dir for transcript text (expect none); inspect
  the SQLite file for key columns (expect null/reference); attempt a webview `read_audio_file`
  with a `../` path (expect rejection).
- **Manual smoke (Definition of Done layers 3–4):** launch bundled `Vinyl.app`, record a
  real meeting (system + mic), confirm live transcript, stop, confirm the meeting + full
  transcript + summary persist, and confirm a forced falling-behind condition surfaces a
  warning instead of silently dropping audio.
- **CI self-check:** open a PR that intentionally introduces a clippy warning and a
  lockfile drift; confirm both fail the pipeline.

---

## Implementation status — 2026-07-01 (branch `audit/robustness-hardening`)

Implemented by the sub-agent team across 7 commits. **All gates green:** `cargo check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test` (367 lib + db_lifecycle +
pipeline_integration + diarization_persistence), `pnpm lint`, `pnpm test` (87/87).
The `vad_filter` adversarial test remains the pre-existing platform failure (CI-excluded).

### Done
- **Critical:** recording-save-on-timeout (frontend); Core Audio tap survives transient
  overflow; bounded queues + drop policy + `transcription-falling-behind`; processed-vs-skipped
  chunk accounting (audio worker) + `ChunkAccounting`/`summary_status` (llm) + retry-with-backoff.
- **High:** transcript no longer logged to disk; fs-confinement on `read_audio_file`/`save_transcript`;
  removed `fs:*-all` + stray CSP entry; Rust-side analytics consent; UTF-8 search panic; DB pool
  busy_timeout+WAL+FK; no-panic DB-init; RT AtomicWaker; system-tap teardown; VAD-init Result;
  Claude max_tokens; per-provider context sizing; low temperature all providers; poll-cleanup;
  notes-flush-on-unmount; CI `-D warnings`+fmt+audit, toolchain pin, pruned meetily workflows.
- **Medium/Low:** most items (see commits) — VAD sinc resampler, bounded live diarization,
  static-mut→atomic, alert()→toast, design tokens, dead-code removal, injection guards,
  script-aware token count, recursive combine, a11y, dev-gated logging, etc.

### Deferred / follow-ups (need owner attention)
- ~~**API-key Keychain migration**~~ — ✅ done in `specs/0030` WS1 (ADR-0009): Keychain-primary
  with sentinel + graceful DB fallback; unblocked once v1.3.0 shipped real Developer ID signing.
- ~~**API-key getter return shape**~~ — ✅ done in `specs/0030` WS2: getters return
  `{ configured, masked }`; all frontend read sites switched; keys are write-only over IPC.
- ~~**ffmpeg SHA-256 digests**~~ — ✅ done in `specs/0030` WS3: real per-target digests pinned
  (dual independent downloads); mismatch or missing digest now fails the build.
- ~~**Frontend wiring of new events/fields**~~ — ✅ done in `specs/0030` WS4: coalesced
  toasts + persistent transcript-gap banner for skip/drop events; partial-summary banner for
  `summary_status`.
- **Next.js 14→15** and **reqwest 0.11→0.12** — intentionally not attempted unattended.
- **Mixer timestamp alignment** and **cpal dedicated-thread refactor** — `TODO(0028)` left;
  need a real-audio soak test.
- **CI transcription coverage** — chose "document (CI-green ≠ transcription verified)" over model
  caching; the record→transcript path still needs the manual smoke test.

### Needs manual verification (can't be unit-tested)
The AtomicWaker RT-callback change, system-audio teardown, and bounded-queue capacities need the
Definition-of-Done smoke test on the bundled `Vinyl.app` (record → live transcript → stop →
persist → summary), including a forced falling-behind condition.
