# 0001 — Foundation setup (Claude Code workflow + fork)

- **Status:** Done
- **Owner agent(s):** (orchestrator)
- **Roadmap phase:** Phase 0

## Context / Problem
We decided to fork meetily and enhance it toward a Granola-quality, local-first macOS meeting
assistant ("Vinyl"). Before feature work, we needed a solid working foundation: a clean
fork/repo structure, agent-facing docs, a focused sub-agent team, a spec-driven workflow, and
sensible permissions — so agents can drive implementation against a known-good baseline.

## Goals
- One git repo at the project root, full history, `upstream` remote tracking meetily.
- Agent-facing `CLAUDE.md`, a 5-agent team, slash commands, and permission settings.
- A spec + ADR + roadmap workflow.
- A documented baseline build + smoke test.

## Non-goals
- Rebranding the Tauri bundle (→ 0002). Tech-debt deletion (→ Phase 1). Any feature code.

## Approach
Hard fork + track upstream. Promote the fork to the repo/project root so `.claude/` and
`CLAUDE.md` are discovered and versioned with the code. Preserve meetily's CLAUDE.md as an
upstream reference.

## Design
- Repo: `git fetch --unshallow`; `origin` → `upstream`; fork contents moved to project root;
  meetily `CLAUDE.md` → `docs/upstream/MEETILY_CLAUDE.md`.
- `.claude/agents/`: audio-engineer, rust-core-engineer, frontend-engineer,
  llm-pipeline-engineer, spec-architect.
- `.claude/commands/`: `/spec`, `/check`, `/run-mac`, `/build-mac`, `/sync-upstream`.
- `.claude/settings.json`: dev-loop allowlist; deny `git push` and reading `.env`.
- `specs/` (template + 0001–0003), `docs/decisions/` (ADR-0001–0003), `ROADMAP.md`.

## Tasks
1. [x] Unshallow, set `upstream`, promote fork to root, relocate upstream CLAUDE.md.
2. [x] Write root `CLAUDE.md`.
3. [x] Create the 5 sub-agents.
4. [x] Add slash commands + `settings.json`.
5. [x] Create specs/ADRs/roadmap scaffolding.
6. [~] Verify baseline build + smoke test on the user's Mac (Phase 0 exit criteria).
   - [x] Toolchain: installed Rust 1.96 + cmake 4.3.4 (Node/pnpm, Xcode CLT, Ollama present).
   - [x] Discovered + documented the **llama-helper sidecar** requirement (must be built via
         `dev-gpu.sh`/`build-gpu.sh`; bare `cargo build`/`clean_run.sh` fail without it).
   - [x] **App compiles end-to-end** headlessly: `cargo build --features metal` → exit 0,
         130MB `meetily` debug binary, with `binaries/{llama-helper,ffmpeg}-<triple>` placed.
   - [ ] Live smoke test (user-driven): launch `./dev-gpu.sh`, grant mic + screen-recording,
         record a short Zoom clip → live transcript → generate summary (local Ollama model).

## Acceptance criteria
- `git remote -v` shows `upstream`; repo root holds the app + our meta-layer.
- `/CLAUDE.md`, the 5 agents, the 5 commands, and `settings.json` exist.
- App builds and runs via `./clean_run.sh`; a test recording yields a live transcript and a
  summary (task 6).

## Verification
See `/CLAUDE.md` → Definition of Done and `ROADMAP.md` Phase 0 exit criteria.
