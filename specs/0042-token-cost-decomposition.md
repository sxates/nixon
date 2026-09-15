# 0042 — Token-cost decomposition (hotspot refactor)

- **Status:** In progress — WS1/WS3/WS4/WS5/WS6 implemented; WS2 (generated IPC contract)
  awaiting the spike + depth decision
- **Owner agent(s):** rust-core-engineer (WS1–WS4) + frontend-engineer (WS2 bindings, WS5)
- **Roadmap phase:** Engineering health (parallel track)

## Context / Problem

A churn analysis of the Vinyl era (357 commits since fork point `df6e341`, 41 shipped
specs) shows that per-change agent token cost concentrates in a handful of large,
frequently-touched files and in the hand-maintained IPC contract:

| Rank | File | Touches | LOC | What it is |
|---|---|---|---|---|
| 1 | `src-tauri/src/lib.rs` | **52** (1 in 7 of all commits) | 1,109 | 218-line `generate_handler!` list + ~370 lines of inline recording commands |
| 2 | `src-tauri/src/api/api.rs` | 25 | 2,109 | grab-bag module: meeting DTOs + 22 commands across domains |
| 3 | `database/repositories/meeting.rs` | 23 | 1,903 | CRUD + snippets + durations + list shaping (274 LOC at fork) |
| 4 | `diarization/{pipeline,commands,sherpa}.rs` | 53 combined | ~4,900 | recently stabilized by 0039 — **leave alone** (see Non-goals) |
| 5 | `src/app/page.tsx` | 28 | 1,047 | home-page god-component |
| 6 | `meeting-details/page-content.tsx` | **32** | 613 | highest-touch frontend file |
| 7 | `src/app/record/page.tsx` | 21 | 671 | recording god-page |

Structural findings behind the numbers:

1. **`lib.rs` is a registration bottleneck.** Nearly every feature edits it just to append
   to the 257-command `generate_handler!` list; it also still hosts inline recording
   commands that belong in `audio/`.
2. **The IPC contract has no single source of truth.** 257 registered commands,
   325 `invoke()` sites, 76 `listen()` sites; frontend types are largely re-declared ad hoc
   at call sites (`types/index.ts` co-changed with Rust in only 10 commits). 34 of the 52
   `lib.rs` commits also touched the frontend — cross-layer edits are the norm, and agents
   must hold both layers in context to keep them consistent by hand.
3. **The god-files are append-magnets.** Each feature appends to them, so every future
   feature reloads them, bigger.

Goal of this spec: attack token cost *structurally* — fewer forced cross-file edits, less
context to load per change — without changing behavior. Zero user-visible change.

## Goals

- Adding a new Tauri command no longer requires editing `lib.rs` (or any >300-line file).
- A generated, typed IPC layer: TypeScript types + typed invoke bindings derived from the
  Rust commands, replacing hand-maintained parallel declarations.
- No source file in the hotspot set above (except the diarization cluster) exceeds ~800
  lines when this spec is done.
- A ratcheting guardrail so files don't regrow: CI flags any source file over the
  threshold unless it's on a shrinking allowlist.
- All refactors are behavior-preserving and covered by the existing gates
  (`cargo check` / `clippy` / `test`, `pnpm lint` / `test`, smoke path).

## Non-goals

- **No behavior, schema, or UI changes.** Pure decomposition/reorganization.
- **No diarization refactor.** `diarization/*` scores high on churn but was just
  stabilized by spec 0039 and has dedicated test coverage; touching it now buys risk, not
  savings. Revisit only if it stays hot after 0039 settles.
- **No conversion to Tauri plugins.** `tauri::plugin::Builder` per subsystem would
  namespace every command (`plugin:audio|start_recording`) and force rewriting all 325
  frontend `invoke` strings. The registry extraction in WS1 gets ~all of the token win
  without that migration. Reconsider only if/after WS2's generated bindings make call
  sites a non-issue.
- **No Next.js → Vite migration, no dependency bumps** (tracked separately on the
  roadmap's engineering-health list).

## Approach

Six workstreams, ordered by token-savings-per-effort. WS1 and WS3/WS4/WS5 are mechanical
and independently landable; WS2 (generated IPC contract) is the big structural win but the
riskiest and gets its own integration branch. WS6 locks in the result.

Sequencing: WS1 → WS3 → WS4 (Rust, sequential — they touch overlapping files) in parallel
with WS5 (frontend, independent); then WS2; WS6 last.

## Design

### WS1 — Decentralize command registration (retires hotspot #1)

- Extract the `generate_handler![...]` list from `lib.rs` into a new
  `src-tauri/src/registry.rs` exposing `pub fn invoke_handler()`. `lib.rs` calls it once
  and stops changing when commands are added. The registry file is a near-pure list —
  cheap to load, trivial to append.
- Evict the inline commands in `lib.rs` (`start_recording`, `stop_recording`,
  `is_recording`, `get_transcription_status`, `read_audio_file`, `save_transcript`,
  audio-level monitoring trio, device/permission/language commands, plus helpers
  `allowed_fs_roots` / `confine_to_roots`) into `audio/` (most belong in or beside
  `audio/recording_commands.rs`; fs-root confinement into its own small module, e.g.
  `src/fs_guard.rs`, since `read_audio_file`/`save_transcript` are not audio-specific).
- Target: `lib.rs` ≤ ~350 lines of stable setup (state init, plugins, tray, migrations).

### WS2 — Generated IPC contract (retires structural finding #2)

- Adopt **tauri-specta v2** (Tauri 2 compatible): `#[specta::specta]` on all 257 commands,
  `specta::Type` derive on every IPC-crossing DTO, `collect_commands![]` builder, and TS
  export to `frontend/src/bindings.ts` (generated in debug builds; committed; CI check
  that it's fresh).
- Registration composes with WS1's `registry.rs` (specta's builder produces the
  invoke handler there).
- Frontend migration is **incremental**: generated bindings replace raw
  `invoke("name", {...})` call sites module-by-module; new code must use bindings.
  Events (76 `listen` sites) migrate to specta-typed events opportunistically.
- Fallback if specta hits a wall (exotic types, macro conflicts with sqlx/tauri):
  **ts-rs** on DTOs only — types become generated but invoke stays stringly. Decision
  gate: a 1-day spike annotating the `audio/` + `api/` command sets end-to-end.

### WS3 — Split `api/api.rs` by domain (retires hotspot #2)

- DTO shaping (`Meeting` view-model, snippet/duration assembly) moves next to
  `database/repositories/meeting.rs`'s domain (e.g. `src/meetings/view.rs`).
- The 22 commands disperse to their feature modules (meetings, settings, search,
  transcripts, participants). `api/api.rs` is deleted when empty.

### WS4 — Split `database/repositories/meeting.rs`

- Split into `meeting/crud.rs` (insert/update/delete/get) and `meeting/query.rs`
  (list shaping, snippets, durations, joins), preserving the existing
  `MeetingsRepository` public surface via re-exports so the 15 unit-tested repo files and
  integration suites don't churn.

### WS5 — Decompose frontend god-pages

- `src/app/page.tsx` (1,047), `meeting-details/page-content.tsx` (613, 32 touches),
  `src/app/record/page.tsx` (671): extract data/IPC logic into hooks
  (`useMeetingList`, `useMeetingDetails`, …) and split presentational sections into
  components under the existing component folders. Match current file/naming conventions
  (`hooks/useRecordingStart.ts` et al. are the pattern).
- Target: each page file ≤ ~300 lines of composition.

### WS6 — File-size ratchet guardrail

- `scripts/check-file-size.sh`: fails if any `.rs`/`.ts`/`.tsx` source file (tests and
  generated files excluded — notably `bindings.ts`) exceeds **800 lines** and is not in
  `scripts/file-size-allowlist.txt`; also fails if an allowlisted file *grows*.
  Allowlist seeded with the current offenders not addressed by this spec (the diarization
  cluster, `calendar/google/sync.rs`, `audio/recording_saver.rs`, `summary/processor.rs`,
  `audio/pipeline.rs`, …) and may only shrink.
- Wired into `.github/workflows/ci-checks.yml` and the `/check` skill.

### Data model

None. No migrations.

### Tauri IPC

No new commands or events. Registration moves to `registry.rs` (WS1); commands gain
`#[specta::specta]` annotations and DTOs gain `specta::Type` (WS2). Command *names and
payloads are unchanged* — the frontend keeps working mid-migration.

### UI

No visual changes. WS5 restructures `page.tsx`, `meeting-details/page-content.tsx`,
`record/page.tsx` into hooks + components; WS2 introduces `src/bindings.ts` and
progressively replaces raw `invoke`/`listen` call sites.

## Tasks

1. [x] **WS1** Extract `registry.rs`; evict inline commands from `lib.rs` into a new
       `audio/capture_commands.rs` + new `fs_guard.rs`; `lib.rs` 1,109 → 404 lines
       (remainder is genuinely stable setup). *(rust-core-engineer, `d69ef61`)*
2. [x] **WS3** Disperse `api/api.rs` commands + DTOs to feature modules
       (`meetings/{view,commands,discard}.rs`, `settings/`, `search.rs`,
       `transcripts.rs`); file deleted. *(rust-core-engineer, `bf41e98`)*
3. [x] **WS4** Split `repositories/meeting.rs` into `meeting/{crud,query,series}.rs`
       behind the existing public surface (series glue emerged as a natural third
       grouping). *(rust-core-engineer, `b7dd4b7`)*
4. [x] **WS5** Decompose the three god-pages into 7 hooks + 11 components
       (277/293/223 lines). Visual spot-check done via `/run-vinyl` (home, record,
       meeting-details all render unchanged). *(frontend-engineer, `65361eb`)*
5. [ ] **WS2a** Spike: tauri-specta on `audio/` + `api/`-successor commands end-to-end;
       go/no-go vs ts-rs fallback. *(rust-core-engineer + frontend-engineer)*
6. [ ] **WS2b** Full rollout: annotate all commands/DTOs, generate + commit
       `bindings.ts`, CI freshness check, migrate call sites module-by-module.
7. [x] **WS6** `check-file-size.sh` ratchet + allowlist (seeded with 31 offenders after
       the splits); wired into CI (`rustfmt` job) and `/check`. *(`1e13124`, `e16f710`)*
8. [x] Update `CLAUDE.md` (architecture map, DoD gate 2b, new-command recipe),
       `specs/TEMPLATE.md` (IPC registration pointer), `specs/INDEX.md`.

Landed alongside (not a WS): workspace-wide `cargo fmt` (`9d15b9b`) — the Rustfmt CI job
had been red on every PR since 2026-07-07 because unformatted code reached `main` via
direct release merges (CI is PR-only); the gate is real again.

## Acceptance criteria

- Definition of Done passes (CLAUDE.md): `cargo check`/`clippy`/`test` clean,
  `pnpm lint`/`test` clean, app launches, record → live transcript → summary smoke intact.
- `git diff --stat` for WS1/WS3/WS4 shows moves/splits only — no logic edits beyond
  visibility/imports (reviewer spot-check).
- Adding a demo command touches exactly: the owning module + `registry.rs` (+ regenerated
  `bindings.ts` under WS2) — demonstrated once in review, then reverted.
- `lib.rs` ≤ 350 lines; `api/api.rs` gone; `meeting.rs` split; each god-page ≤ ~300 lines.
- WS2: `bindings.ts` generated and fresh in CI; at least the audio/recording +
  meetings call sites use generated bindings; no `any`-typed invoke wrappers added.
- WS6 ratchet green in CI with the seeded allowlist.

## Risks / open questions

- **TODO (the owner): WS2 depth.** Full tauri-specta rollout (recommended — biggest
  structural win) vs. ts-rs types-only (half the value, quarter the risk). The WS2a spike
  answers feasibility; the depth choice is yours.
- **TODO (the owner): guardrail threshold + severity.** 800 lines proposed; hard-fail in CI
  vs. warn-only for the first release?
- **TODO (the owner): release cadence.** Land WS1/WS3/WS4/WS5 in one release (v1.10.0) and
  WS2 in the next, or batch everything?
- Move-refactors can silently change behavior via `pub` visibility, module-scoped
  `thread_local!`/statics (the language-preference statics in `lib.rs` are one), or
  `#[cfg]` scoping — mitigated by the move-only diff review rule and the test gates.
- specta derive may fight sqlx/serde attribute macros on shared models — surfaces in the
  WS2a spike before any commitment.
- Merge friction: WS1/WS3 touch every command's registration line — land them quickly,
  not in parallel with feature branches.

## Verification

- Per-WS: `cd frontend/src-tauri && cargo check --features metal && cargo clippy && cargo test`
  (incl. the mic-free suites: `transcription_engine`, `vad_filter`, `pipeline_integration`,
  `db_lifecycle`); `cd frontend && pnpm lint && pnpm test`.
- App smoke via `./clean_run.sh` / `/run-vinyl`: record → live transcript → summary; open
  meeting details; home + record pages screenshot-compared before/after WS5.
- WS2: a round-trip type test — a command whose DTO changes in Rust must fail
  `pnpm tsc --noEmit` until bindings are regenerated (proves the contract is live).
- WS6: intentionally grow an allowlisted file by one line in a scratch branch → CI fails.
