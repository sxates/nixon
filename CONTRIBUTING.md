# Contributing to Nixon

Nixon is a **local-first, on-device meeting assistant for macOS** (Tauri 2 + Next.js 14 +
React 18, ~44k LOC of Rust). It records meetings, transcribes them in real time, and
summarizes them — and **nothing leaves the machine** except calls to a user-chosen LLM
provider. Privacy is the product; keep it that way.

This repo is a hard fork of [meetily](https://github.com/Zackriya-Solutions/meetily) v0.4.0
(MIT), tracked as the git remote `upstream`. We diverge freely but cherry-pick their audio
fixes. Read [`CLAUDE.md`](CLAUDE.md) first — it is the authoritative architecture + build
guide; this file is the contribution workflow.

## Branch & PR flow

- **`main` is the single long-lived branch.** There is no `devtest` branch (that was
  upstream meetily's flow — ignore any leftover references to it).
- Cut a **feature branch off `main`** for every change; open a PR back into `main`.
- Keep diffs small and reviewable. Every feature has a numbered spec in `specs/`; land the
  change against its spec and check the spec's task boxes as slices go green.
- Architectural choices get a short ADR in `docs/decisions/`.
- Run the built-in `/code-review` before handing a change back.

## Build & run (macOS / Metal)

```bash
cd frontend
pnpm install
./dev-nixon.sh       # DEV launcher → runs as "Dev Nixon" (ai.vinyl.app.debug), isolated data
./build-gpu.sh       # production build (also builds the llama-helper sidecar)
./upgrade-nixon.sh   # rebuild + reinstall /Applications/Nixon.app, preserving data
```

Requires **microphone** and **screen-recording** permission (the latter is what lets the
Core Audio tap capture system/Zoom audio — no BlackHole needed). Dev and production are
separate apps with separate data (ADR-0004).

### Two gotchas that bite everyone

1. **cargo must be on PATH.** `dev-gpu.sh` / `build-gpu.sh` are upstream `#!/bin/bash`
   scripts that don't source rustup's env, so calling them directly fails with
   `cargo: command not found`. Use **`./dev-nixon.sh`** (it sources `~/.cargo/env` first,
   then delegates to `dev-gpu.sh`, which is kept unmodified to stay mergeable with upstream).
2. **The `llama-helper` sidecar.** The app bundles two `externalBin` binaries declared in
   `tauri.conf.json`: `binaries/llama-helper-<triple>` and `binaries/ffmpeg-<triple>`.
   `build.rs` auto-downloads ffmpeg (now with SHA-256 verification scaffolding in
   `src-tauri/build/ffmpeg.rs`), but **nothing auto-builds `llama-helper`** except
   `dev-gpu.sh` / `build-gpu.sh`. A bare `cargo build`, `pnpm tauri:dev`, or `clean_run.sh`
   on a clean tree fails with `resource path 'binaries/llama-helper-...' doesn't exist`. The
   `pretauri:dev` / `pretauri:build` npm hooks now fail early with a clear message pointing
   you at the right script; run `./dev-nixon.sh` (dev) or `./build-gpu.sh` (prod) once and
   the simpler paths work thereafter.

## Definition of Done (the gate — run `/check`)

A change is done when all of these are green:

1. **Rust** (in `frontend/src-tauri`, with `source ~/.cargo/env`):
   ```bash
   cargo fmt --check
   cargo check --features metal
   cargo clippy --features metal --all-targets -- -D warnings
   cargo test --features metal --lib --test db_lifecycle --test diarization_persistence
   ```
   `clippy` runs with **`-D warnings`** — a new warning fails the build. Don't silence it;
   fix or remove the offending code.
2. **Frontend** (in `frontend`):
   ```bash
   pnpm lint
   pnpm test
   ```
3. The app still launches via `./clean_run.sh`.
4. The relevant smoke path still works: **record → live transcript → summary**.

Layers 1–2 (the deterministic subset) run automatically on every PR via
`.github/workflows/ci-checks.yml`. Layers 3–4 are **manual** — see the caveat below.

### CI-green does NOT mean transcription is verified

The record → live-transcript → summary critical path depends on the Parakeet/diarization
models, which are **not** downloaded on CI runners. So:

- The model-gated `transcription_engine` test **skips** in CI.
- `vad_filter` / `pipeline_integration` synthesize audio via macOS `say`, whose output (and
  thus the VAD thresholds) varies by OS version, so they run **locally**, not as a CI gate.

CI proves the deterministic subset (lib unit tests + `db_lifecycle` +
`diarization_persistence`) plus fmt/clippy/lint stay green. The **end-to-end transcription
smoke test (Definition of Done #4) is your responsibility to run manually** against the
bundled `Nixon.app` before shipping anything that touches audio/STT/summary. Run the full
local suite per `CLAUDE.md`:

```bash
cd frontend/src-tauri && source ~/.cargo/env
cargo test --features metal --test transcription_engine --test vad_filter \
  --test pipeline_integration --test db_lifecycle
```

## Toolchain pins

- **Rust:** `rust-toolchain.toml` at the repo root pins the channel + `clippy`/`rustfmt`
  components. Crate MSRV is 1.77 (edition 2021).
- **Node:** `frontend/.nvmrc` pins Node 20; `nvm use` in `frontend/`.
- **pnpm:** `frontend/package.json` `packageManager` pins pnpm 11 (matches
  `pnpm-lock.yaml` lockfileVersion 9.0). Use `pnpm install --frozen-lockfile`; commit
  lockfile changes deliberately.

## Releasing

**`release.sh` at the repo root is the single source of truth** for cutting a release. It
bumps the version across `package.json` / `tauri.conf.json` / `Cargo.toml` / `Cargo.lock`,
rolls `CHANGELOG.md`, builds + notarizes the macOS DMG, merges to `main`, tags
**`nixon-vX.Y.Z`**, and publishes a GitHub Release with the DMG attached:

```bash
./release.sh <major|minor|patch|X.Y.Z> [--skip-build] [--no-release] [--yes] [--dry-run]
```

> The old meetily GitHub-Actions release/build workflows (`release.yml`, `build.yml`, the
> `build-*.yml` matrix, `pr-main-check.yml`) were **removed** (specs/0028): they were
> mis-branded ("Meetily v…"), tagged `v${version}` instead of our prefixed tag, and referenced an
> `s3://meetily-updates` updater that `tauri.conf.json` never wired up. Nixon is macOS-only;
> `release.sh` is the real path. `.github/workflows/ci-checks.yml` is the only remaining
> (and only auto-running) workflow.

## Code style & conventions

- **Rust:** `anyhow::Result`; Tauri command (frontend→Rust) + event (Rust→frontend) pattern;
  audio devices are named `"microphone"` / `"system"` (never `"input"`/`"output"`). Never
  hardcode paths — derive them from the identifier dir via `src/app_paths.rs` / Tauri path
  APIs.
- **Frontend:** Tauri `invoke` / `listen` IPC; user-friendly try/catch error messages;
  prefer design tokens over hardcoded Tailwind palette classes.
- **Don't** reintroduce the archived Python/FastAPI `backend/` as a runtime dependency
  (legacy reference only), and **don't** build on the stub `audio_v2/` refactor.
- **Commits:** conventional style (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `chore:`), imperative subject.

## License

By contributing, you agree that your contributions are licensed under the project's MIT
License.
