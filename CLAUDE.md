# CLAUDE.md — Nixon

Guidance for Claude Code and sub-agents working in this repo.

> **`APP_NAME` = "Nixon"** — the product name is a *variable* (ADR-0002). Rebrands so far:
> meetily → Vinyl (`specs/0002`, 2026-06) → Nixon (`specs/0057`, 2026-09). The bundle id is
> still **`ai.vinyl.app`** (dev `ai.vinyl.app.debug`) — deliberately kept so no permission
> re-grant or data migration was needed; it is invisible to users and will move in a future
> identifier change (which must run `src/data_migration.rs` again). `APP_NAME` lives in
> `src/app_paths.rs`.

## What we're building

Nixon is a **local-first, on-device meeting assistant** for macOS: it records meetings
(Zoom/Meet/Teams — any system audio + mic), transcribes them in real time, summarizes them,
and organizes them for later reference and analysis. The quality bar is **granola.ai**.

**Privacy is the whole point:** meeting audio, transcripts, and notes never leave the
machine. The only outbound traffic is to a *user-chosen* LLM provider for summarization
(and the default should be a local model via Ollama).

## Fork relationship (read this before touching code)

This repo is a **hard fork of [meetily](https://github.com/Zackriya-Solutions/meetily)
v0.4.0** (MIT). Upstream is tracked as the git remote **`upstream`**; we diverge freely but
cherry-pick their audio fixes (see `/sync-upstream`).

**What we KEEP from meetily (it's good — don't rewrite):**
- The macOS audio capture + mixing + VAD pipeline.
- Real-time transcription (Whisper.cpp / Parakeet) with Metal/CoreML acceleration.
- SQLite persistence (sqlx + migrations), the Tauri desktop shell, multi-provider LLM plumbing.

**What we REBUILD / ADD (this is our product):**
- The Granola-style **note-enhancement** flow (live notepad + AI that enriches *your* notes
  with the transcript) — our flagship differentiator. See `specs/0003`.
- **Speaker diarization** (meetily has none; only "mic" vs "system" labels).
- **Search & organization** (full-text search, tags/folders, cross-meeting analysis).

**Do NOT** reintroduce the archived Python/FastAPI `backend/` as a runtime dependency — it's
legacy reference only. The supported app is entirely the Tauri/Rust core under
`frontend/src-tauri/`.

Upstream's own architecture notes are preserved at `docs/upstream/MEETILY_CLAUDE.md` — a
useful deep reference for how the existing code works.

## Architecture map (file anchors)

The app lives under `frontend/` (Tauri 2 + Next.js 14 + React 18). ~44k LOC of Rust.

| Subsystem | Where | Notes |
|---|---|---|
| Tauri entry / commands | `frontend/src-tauri/src/lib.rs` + `registry.rs` | `lib.rs` = stable setup only; **new commands register in `registry.rs`** (specs/0042) |
| Meetings / settings commands | `frontend/src-tauri/src/meetings/`, `settings/`, `search.rs`, `transcripts.rs` | former `api/api.rs`, dispersed by domain (specs/0042) |
| Audio capture (macOS) | `frontend/src-tauri/src/audio/capture/core_audio.rs`, `capture/system.rs` | **Core Audio process tap — no BlackHole required**; needs audio-capture permission |
| Audio mixing + VAD | `frontend/src-tauri/src/audio/pipeline.rs`, `audio/vad.rs` | RMS ducking; Silero VAD drops silence |
| Transcription (STT) | `frontend/src-tauri/src/whisper_engine/`, `parakeet_engine/`, `audio/transcription/` | Whisper.cpp (whisper-rs) + Nvidia Parakeet; real-time/streaming |
| Summarization | `frontend/src-tauri/src/summary/processor.rs`, `service.rs`, `llm_client.rs`, `templates/` | chunking + template-fill report |
| LLM providers | `frontend/src-tauri/src/{ollama,anthropic,openai,groq,openrouter}/` | Ollama (local) + cloud |
| Database | `frontend/src-tauri/src/database/` (sqlx) + `frontend/src-tauri/migrations/` | SQLite at `~/Library/Application Support/<bundle-id>/meeting_minutes.sqlite`; bundle id = **`ai.vinyl.app`** (production `Nixon.app`) or **`ai.vinyl.app.debug`** (dev "Dev Nixon") — isolated from each other and from meetily's `com.meetily.ai` (ADR-0004); `api_set_segment_text` (specs/0061 W5) edits this DB only — the recording folder's `transcripts.json` stays the raw, unedited capture |
| Frontend UI | `frontend/src/` | Next.js; BlockNote editor (`components/BlockNoteEditor/`, `AISummary/`) |
| Notes (exists, unused by summary) | `meeting_notes` table; `frontend/src/app/notes/[id]/` | reuse for note-enhancement (`specs/0003`) |

## Build & run (macOS / Metal)

```bash
cd frontend
pnpm install
./dev-nixon.sh           # ← DEV launcher (wraps dev-gpu.sh; cargo on PATH). Runs as "Dev Nixon",
                         #   identifier ai.vinyl.app.debug — ISOLATED data from production.
./dev-nixon.sh --demo        # seed 5 fictional meetings + audio into the DEBUG profile (specs/0059)
                              # first run: ~4 min (say/ffmpeg synthesis); re-running --demo reuses
                              # the cached audio (folders are stable, so it's seconds) — use
                              # --no-audio for fast iteration when audio doesn't matter
./dev-nixon.sh --onboarding  # re-run onboarding with simulated downloads (--real-downloads to keep them)
pnpm shots               # headless screenshot capture of every route, both themes (specs/0060)
pnpm shots:diff          # pixelmatch contact sheet vs HEAD (docs/screenshots/diff/index.html)
pnpm shots:real   # real-window captures; needs ./dev-nixon.sh --demo running + Screen Recording for the terminal
./build-gpu.sh           # production build (also builds the sidecar) — needs cargo on PATH
./upgrade-nixon.sh       # rebuild + reinstall /Applications/Nixon.app, preserving data
```

**Cutting a release: ALWAYS `./release.sh <major|minor|patch> [--yes]` at the repo root —
never hand-roll the bump/tag/`gh release` steps.** It sources the gitignored `.env.signing`
(Developer ID + notarization creds; see ADR-0008 + SETUP.md) so the DMG is signed,
notarized, and stapled — a DMG built any other way ships ad-hoc-signed and Gatekeeper
blocks it on download (this has shipped twice: v1.3.0, v1.10.0). It also rolls the
changelog ([Unreleased] must be non-empty), merges/tags (`nixon-vX.Y.Z`), pushes, and
publishes the GitHub release with the DMG attached. `tauri.conf.json`'s
`signingIdentity: "-"` (ad-hoc) is the deliberate fallback — don't hardcode an identity there.

**Dev vs production are separate apps with separate data (ADR-0004).** The production app is the
bundled **`Nixon.app`** (identifier `ai.vinyl.app`) — the user's real meetings. The **dev** build
(`./dev-nixon.sh`) runs as **"Dev Nixon"** under `ai.vinyl.app.debug` (set via
`src-tauri/tauri.dev.conf.json`, merged by `scripts/tauri-auto.js` for the `dev` command only), so
testing **never** touches production data. Both can run at once; the dev build shows a **"DEV" badge**
in the sidebar. Upgrade the production app with `./upgrade-nixon.sh` (data lives in
`~/Library/Application Support/ai.vinyl.app/` plus the recordings folder, keyed by identifier,
so replacing the `.app` never loses it; migrations are forward-only). **Recordings folder:**
fresh installs use `~/Movies/nixon-recordings/`. An install that has
`~/Movies/meetily-recordings/` keeps writing there for as long as that folder exists
(`audio/recording_preferences.rs` prefers the legacy folder whenever it is present, even if a
`nixon-recordings` folder appears beside it), so nothing is moved or re-pointed. On such a
machine the debug build's recordings root is that same legacy folder, and `--demo`'s
`nixon-demo-*` folders land there too — cleanup only ever deletes a folder that both starts
with the `nixon-demo-` prefix AND has a `metadata.json` whose `meeting_id` starts with
`demo-` and whose microphone device is `"Demo Microphone"`, so it can never touch a real
recording folder even if one happened to share the prefix. Also note: `--onboarding`
simulates the model downloads only (no bytes hit disk), so recording after a simulated
onboarding still needs the real models downloaded and present on disk.
**Don't call `dev-gpu.sh` / `build-gpu.sh` directly unless cargo is already on PATH.** They're
upstream `#!/bin/bash` scripts that don't source rustup's env, so they fail with
`cargo: command not found`. `dev-nixon.sh` sources `~/.cargo/env` first, then delegates to
`dev-gpu.sh` (kept unmodified to stay mergeable with upstream).
**Gotcha — the sidecar:** the app bundles two external binaries declared in
`tauri.conf.json` `externalBin`: `binaries/llama-helper-<triple>` (the BuiltInAI local-model
helper, a workspace member) and `binaries/ffmpeg-<triple>`. `build.rs` auto-downloads ffmpeg,
but **nothing auto-builds `llama-helper`** — only `dev-gpu.sh` / `build-gpu.sh` do (via
`cargo build` in `llama-helper/` then copy to `frontend/src-tauri/binaries/`). A bare
`cargo build`, `pnpm run tauri:dev`, or `clean_run.sh` will **fail** with
`resource path 'binaries/llama-helper-...' doesn't exist` on a clean tree. Once the sidecar
is present, those simpler paths work too. Note: `llama-cpp-2` has no CoreML feature, so the
sidecar uses the `metal` feature on Apple Silicon.

**Identifiers / isolation:** production `Nixon.app` = `ai.vinyl.app`; dev "Dev Nixon" =
`ai.vinyl.app.debug`; installed upstream meetily = `com.meetily.ai`. All three have separate
SQLite DB, settings, single-instance lock, and TCC permissions, so any combination can run at
once without touching each other's data. (The former hardcoded `.../Meetily/templates/` leak is
fixed — all storage now derives from the identifier dir via `src/app_paths.rs`.) Each new
identifier prompts fresh for mic + audio-capture on first run. **Dev signing:** `tauri dev`
builds through `src-tauri/scripts/cargo-dev-sign.sh` (the `build.runner` in `tauri.dev.conf.json`),
which re-signs the debug binary (`target/debug/nixon`) with a stable Apple Development identity so
Keychain ACLs (ADR-0009 API keys) and the Audio-Capture grant survive rebuilds — previously the
per-rebuild ad-hoc signature orphaned both (password prompts; system-audio tap silently returning
silence). On a machine with no Apple Development cert it no-ops back to ad-hoc, where those two
gotchas still apply. See ADR-0004 §Dev-build gotchas.

Requires microphone **and** audio-capture permission (the latter is what lets the Core
Audio tap capture system/Zoom audio). System audio capture does **not** need BlackHole.

For a Rust-only compile check (no GUI), `cd frontend/src-tauri && cargo build --features metal`
works **once the sidecar binary exists**.

## Definition of Done (the gate — run `/check`)

A change is done when:
1. `cargo check`, `cargo clippy`, and `cargo test` are clean (in `frontend/src-tauri`).
2. `pnpm lint` and `pnpm test` are clean (in `frontend`).
2b. `scripts/check-file-size.sh` passes (specs/0065): no production source file over 800
   lines unless it is in `scripts/file-size-tracked.txt`, **and** the total excess across
   those grandfathered files — `sum(lines - 800)` — stays within
   `scripts/file-size-budget.txt`. The budget is shared, so a small addition to a large
   legacy file is fine when there is headroom; pay it down anywhere in the tracked set, not
   necessarily in the file you touched. `--update` drops files that fell under the cap and
   lowers the budget; it never raises either. `--self-test` runs the gate's own checks.
   (This replaces the specs/0042 per-file ratchet, which froze each allowlisted file at its
   exact size and ended up blocking one-line `use` statements.)
2c. `scripts/check-off-token-colors.sh` passes (specs/0057: no raw Tailwind palette classes —
   everything goes through the semantic tokens so Deck/Faceplate stay in parity).
2d. For UI changes, run `pnpm shots` then `pnpm shots:diff` from `frontend/` and look at the
   sheet (specs/0060).
3. The app still launches via `./clean_run.sh`.
4. The relevant smoke path still works (record → live transcript → summary).

Layers 1–2 run automatically on every PR via `.github/workflows/ci-checks.yml`; the test
strategy and the recording-lifecycle regression coverage live in `specs/0023`.

**Audio/transcription tests (no mic needed):** the recording pipeline below the hardware
capture layer (mix/VAD/pipeline/STT/DB) is covered by `cargo test` fixtures in
`frontend/src-tauri/tests/` — run `cd frontend/src-tauri && source ~/.cargo/env && cargo test
--features metal --test transcription_engine --test vad_filter --test pipeline_integration
--test db_lifecycle`. The Parakeet engine test skips cleanly when the model isn't downloaded;
fixtures are generated at test time via macOS `say`. See `specs/0009`. (The Core Audio
tap/mic itself still needs the manual smoke test in #4.) The fixture seeder is covered by
`cargo test --features metal --test dev_fixtures_seed`.

## How we work

- **Agents drive, the user reviews.** Implement against a spec, keep diffs reviewable, and
  run `/code-review` before handing back.
- **Spec before code.** Every feature gets a numbered spec in `specs/` (scaffold with
  `/spec`). Architectural choices get a short ADR in `docs/decisions/`.
- **The Phase 1 tech-debt cleanup is done — keep it that way:** the leftover `*_old.rs`
  files, the `audio_v2/` half-refactor stub, and the archived Python `backend/` were deleted
  (CHANGELOG `[0.2.0]` Removed). Don't resurrect them from git history; `audio/` is the
  supported pipeline.

### Sub-agent team (`.claude/agents/`)

| Agent | Use it for |
|---|---|
| `audio-engineer` | audio capture/mixing/VAD, Whisper/Parakeet, GPU accel, **diarization** |
| `rust-core-engineer` | Tauri commands/state, sqlx DB, summary orchestration, LLM providers |
| `frontend-engineer` | Next.js/React/TS, BlockNote editor, Tauri IPC, recording & notes UX |
| `llm-pipeline-engineer` | prompts, summarization & **note-enhancement** logic, templates, evals |
| `spec-architect` | writing specs, decomposing features, ADRs |

Reviews: use the built-in `/code-review` skill.

### Conventions

- Rust: `anyhow::Result`; Tauri command (frontend→Rust) + event (Rust→frontend) pattern;
  audio devices named "microphone"/"system" (never "input"/"output").
- Frontend: Tauri `invoke`/`listen` IPC; user-friendly try/catch error messages.
- Never hardcode paths — use Tauri path APIs for cross-platform/app-data locations.
- Git: feature branches off `main`; 
