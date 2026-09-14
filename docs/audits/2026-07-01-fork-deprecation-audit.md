# Fork-Divergence Deprecation Audit

**Date:** 2026-07-01
**Scope:** What did we inherit from meetily v0.4.0 that Vinyl no longer needs?
**Method:** Three parallel read-only sweeps (cross-platform/scripts/CI, Rust core, frontend
surface), each verified against the tree with grep/`git log` evidence; every "no caller"
claim below was checked against string-literal `invoke(...)` call sites (the frontend has no
dynamic invoke wrappers, so static analysis is reliable). Conflicting findings were
re-verified by hand. Baseline: ~60k LOC tracked Rust, ~44.5k LOC frontend TS/TSX.

---

## Executive summary

The big fork cleanup already happened — 0.2.0 deleted the Python backend, `*_old.rs`,
`audio_v2/`, and the auto-updater (~28k lines), and 0.7.1/0028 pruned the meetily CI and
release plumbing. What's left is a second, smaller ring of cruft: **roughly 6,500–7,000 LOC
plus a 4.5 MB committed Windows binary are removable now with near-zero risk (~6–7% of the
product surface), and another ~2–3% sits in a "verify, then delete" tier.** The Rust
cross-platform `#[cfg]` seams (~1,200 gated LOC) look like cruft but are the cheap glue that
keeps upstream audio cherry-picks mergeable — recommended keep.

**Top 5 wins:**

1. **Delete `frontend/vs_buildtools.exe` (4.5 MB) and all 9 `.bat`/`.ps1` build scripts
   (~1,245 LOC).** A Visual Studio installer binary and Windows build scripts in a
   macOS-only repo. Zero references from any live flow. Zero risk.
2. **Delete the orphaned frontend component clusters (~3.5k LOC) and 16 dead npm
   packages.** The old `AISummary` view (1,193 LOC), `TranscriptView` (372), the entire
   remirror+tiptap editor stacks (12 packages), `react-markdown`, `lodash`, the
   updater/process Tauri plugins — all verified zero importers.
3. **Delete the Whisper parallel-processing subsystem (~1,100 LOC, 11 Tauri commands).**
   meetily's batch-transcription path; Vinyl transcribes through
   `audio/transcription/worker.rs`. No frontend caller for any of its 11 commands.
4. **Fix the dual-config landmines the audit surfaced:** `tailwind.config.js` vs
   `tailwind.config.ts` and `postcss.config.js` vs `.mjs` have *conflicting* content and
   only one of each loads — whichever loses is silently dead (typography plugin or
   autoprefixer may not be running today). Not cruft — a live correctness hazard.
5. **Fix a live-but-broken feature:** the console toggle (`show_console` etc., wired to
   `ConsoleToggle.tsx`… which is itself orphaned) runs
   `log stream --process meetily` — a process name that no longer exists
   (`console_utils/console_utils.rs:52,90,132`). Either fix the process name or delete the
   whole console feature with its orphaned UI.

**Headline stat:** 51 of ~249 registered Tauri commands (~20%) have no frontend caller.

---

## Area 1 — Cross-platform code for platforms we don't ship

### 1a. Windows/Linux scripts & binaries — delete now

All git-tracked, none invoked by any live flow (`dev-vinyl.sh`, `build-gpu.sh`,
`upgrade-vinyl.sh`, `release.sh`, CI):

| File | LOC / size | Referenced by |
|---|---|---|
| `frontend/vs_buildtools.exe` | **4.46 MB binary** | `build.ps1`, `build-gpu.ps1` only |
| `frontend/build.bat` | 201 | nothing |
| `frontend/build_backup.bat` | 201 | nothing (a *backup* of build.bat — doubly dead) |
| `frontend/build-gpu.bat` | 253 | nothing |
| `frontend/dev-gpu.bat` | 241 | nothing |
| `frontend/build.ps1` | 76 | nothing |
| `frontend/build-gpu.ps1` | 74 | a comment in `src-tauri/Cargo.toml:36` |
| `frontend/dev-gpu.ps1` | 61 | nothing |
| `frontend/clean_build_windows.bat` / `clean_run_windows.bat` | 11 + 11 | only `docs/upstream/MEETILY_CLAUDE.md` (archive) |
| `frontend/scripts/load-env.ps1` | 78 | `build.ps1` only (dies with it) |
| `frontend/src-tauri/scripts/sign-windows.ps1` | 38 | `tauri.conf.json:113` `bundle.windows.signCommand` |
| `docs/building_in_linux.md` | 341 | `docs/GPU_ACCELERATION.md:17` |

**Subtotal: ~1,586 LOC + 4.46 MB.** Risk: none. Upstream-merge friction: low — leaf files;
future cherry-picks touching them will just report "deleted by us."

Related config trims:
- `tauri.conf.json` `bundle.targets` still lists `deb`, `appimage`, `msi`, `nsis` alongside
  `app`/`dmg`; the `bundle.windows` block (lines ~112–113) invokes the sign script above.
  Trim to `["app","dmg"]` and drop the windows block — note this file is upstream-shared, so
  expect a trivial re-reconcile on merges.
- `frontend/package.json`: 8 non-mac GPU script variants (`tauri:dev:cuda/vulkan/hipblas/openblas`
  and `tauri:build:*` twins). Delete — package.json already diverges heavily from upstream.
- `frontend/src-tauri/Cargo.toml`: the Windows/Linux build-instruction comment block
  (~lines 18–38) is stale for this fork; safe to trim.

### 1b. Rust `#[cfg]` cross-platform code — keep (mostly)

143 cfg attributes across 24 files, ~1,200 gated LOC. Most are
`#[cfg(target_os = "macos")]` real implementations paired with thin
`#[cfg(not(target_os = "macos"))]` stubs — upstream meetily heritage, zero cost in the mac
binary (cfg'd out at compile time), and exactly the seam that keeps upstream audio
cherry-picks mergeable. **Recommendation: keep**, with two exceptions worth a focused pass:

- `src/console_utils/console_utils.rs` — the most Windows-heavy file (8 cfg blocks, ~60–80
  LOC of dead Windows `AllocConsole` externs + Linux stubs), *and* its live macOS arm is
  broken (`--process meetily`, see win #5). If the console feature isn't wanted, delete the
  module + its 3 commands + the orphaned `ConsoleToggle.tsx` in one stroke.
- `src/audio/capture/windows_loopback.rs` (349 LOC WASAPI capture) — dead on mac but pure
  upstream audio code; **keep** per the cherry-pick policy.

The `[target.'cfg(target_os = "windows")']` / `linux` dep blocks in Cargo.toml never compile
on mac — keep as merge seams. `build.rs` / `build/ffmpeg.rs` Windows/Linux ffmpeg URL
branches (~50 LOC): harmless, upstream-shaped, keep.

### 1c. CI — already clean

`.github/workflows/` contains only `ci-checks.yml` (macOS-14 Rust job + ubuntu JS job; the
meetily build/release workflows were pruned in 0028). No action.

---

## Area 2 — The Python backend: deleted, but seven stragglers remain

The `backend/` dir is fully gone (removed in `96a8519` / `2749775`; CLAUDE.md's "archived"
wording is slightly stale — it lives only in git history). What's left pointing at it:

| Straggler | Evidence | Action |
|---|---|---|
| `frontend/src-tauri/config/backend_config.json` | `fastApiEndpoint: localhost:5167`; **no reader** (the similarly named `audio/capture/backend_config.rs` is about audio-capture backends, unrelated — verified) | Delete file |
| `frontend/src/components/Sidebar/SidebarProvider.tsx:125-126` | sets `serverAddress` `localhost:5167` / `127.0.0.1:8178` defaults | Delete dead defaults |
| `tauri.conf.json:30` CSP | `connect-src` still allows `localhost:5167` + `:8178` | Tighten CSP (keep `11434` for Ollama) |
| `src/api/api.rs:233-236` | dead-5167 integration removed but `auth_token: Option<String>` params linger | Drop params (tracked in `specs/0028:295`) |
| `src/whisper_engine/whisper_engine.rs:108-111` | model-path fallback into `backend/whisper-server-package/models` | Delete branch |
| `src/audio/import.rs:1054,1067,1079` | test paths `../../backend/whisper.cpp/samples/jfk.*` — files no longer exist | Fix or drop those tests |
| `frontend/API.md` (23 LOC) | "Legacy Whisper Server API Archive", refers to Meetily | Delete |

`SETUP.md`, `README.md`, `frontend/README.md` are already rewritten and clean (verified — no
python/FastAPI/5167 references). `docs/upstream/MEETILY_CLAUDE.md` mentions the backend by
design — keep.

---

## Area 3 — Dormant DB schema

27 migrations; schema is mostly live. The dormant pieces:

| Item | Evidence | Verdict |
|---|---|---|
| `meeting_notes.enhanced_markdown/json/at/model` | Written by **nothing**; self-documented dormant at `database/repositories/meeting_note.rs:55-57`; only "read" via `SELECT *` into struct fields nothing consumes (`models.rs:251-254`) | **Keep columns** (cheap, harmless, migrations are forward-only) but treat the feature as abandoned; consider dropping in a future schema consolidation |
| `settings.geminiApiKey` (`migrations/20251229000000`) | Zero Gemini references anywhere in Rust; Gemini absent from the `LLMProvider` enum and the frontend picker | Fully dead column — a half-migration. Keep (dropping columns isn't worth a migration), but don't build on it |
| `transcript_chunks` table | **Write-only.** INSERT at `repositories/transcript_chunk.rs:26`; its `transcript_text` is never SELECTed — it appears only in an existence-check JOIN in `summary.rs:78`. Vinyl's real transcript store is `transcripts` | Deprecate; untangle the summary JOIN first, then stop writing |
| `transcripts.summary/action_items/key_points` | meetily-era columns; fork uses `summaries`/`summary_processes` | Dormant, low priority |

`summary_processes`, `settings`, `transcript_settings` are all live (read+write sites
verified). Risk of touching schema: user data + forward-only migrations, so the safe move is
"stop writing / stop reading" now and physically drop columns only if a consolidation
migration ever happens anyway.

---

## Area 4 — Unused Tauri commands (51 of ~249)

Registered in `lib.rs` `generate_handler!` (lines 729–1017). The dead fifth, grouped:

| Group | Count | Notes | Verdict |
|---|---|---|---|
| **Parallel Whisper processing** | 11 | `initialize_parallel_processor`, `start/pause/resume/stop_parallel_processing`, `get_system_resources`, `prepare_audio_chunks`, … Backing code: `whisper_engine/parallel_commands.rs` (266) + `parallel_processor.rs` (480) + `system_monitor.rs` (293) ≈ **1,039 LOC**; `ParallelProcessorState` managed at `lib.rs:533`, consumed by nothing called | **Delete the subsystem** — meetily's batch path; Vinyl uses `audio/transcription/worker.rs`. Low merge friction |
| Notifications surface | ~10 | module itself (1,395 LOC) is live (startup init `lib.rs:592`, recording notifications) but its public command surface is unused by UI | Deprecate commands, keep module |
| System-audio standalone commands | 5–6 | capture happens through the pipeline; these wrappers are uncalled | Deprecate wrappers, **keep the `audio/` code beneath** (upstream merge value) |
| Screen-recording perm cmds | 2 | only `trigger_system_audio_permission_command` is called | Deprecate |
| Device-reconnection | 3 | `poll_audio_device_events` etc. — AirPods reconnect feature registered, never wired to UI | Investigate: unfinished feature or dead? |
| Ollama extras | 2 | `delete_ollama_model`, `get_ollama_model_context` (siblings are used) | Deprecate |
| Analytics | 2 | `track_meeting_ended`, `track_summary_generation_started` (their siblings ARE called) | Delete — or wire them up; the asymmetry looks accidental |
| Misc half-wired | ~11 | `api_validate_template`, `api_clear_segment_speaker`, `select_recording_folder`, state probes (`is_recording_paused`, …), `start_recording_with_devices` (thin wrapper at `lib.rs:362`, only the `_and_meeting` variant is called) | Deprecate; delete the wrapper |

Command removal is cheap and self-contained (handler entry + function), and this is
Vinyl-divergent surface, so upstream friction is minimal.

---

## Area 5 — Unused frontend surface (~4k LOC + 16 packages)

### Routes — clean
All routes are reachable from nav/command-palette except two deliberate cases:
`/design-preview` (dev-only, gated `NODE_ENV`, keep) and `/notes/[id]` (a back-compat
redirect shim to meeting-details per the spec-0003 pivot — keep for now).

### Orphaned components/hooks (zero importers, name-and-path verified)

| Cluster | LOC | Replacement | Verdict |
|---|---|---|---|
| `AISummary/index.tsx` + `Section.tsx` + `Block.tsx` | 1,193 | `AISummary/BlockNoteSummaryView.tsx` (the only live file in the dir) | Delete now |
| `TranscriptView.tsx` | 372 | `VirtualizedTranscriptView.tsx` | Delete now |
| `AudioPlayer.tsx` (0 LOC, empty!) + `hooks/useAudioPlayer.ts` | 275 | — | Delete now |
| `molecules/form-components/*` (3 files) | 281 | `ui/*` primitives | Delete now |
| `ui/sheet.tsx`, `ui/input-group.tsx`, `ui/progress.tsx` | 338 | never imported (note: `.ds-entry.tsx` re-exports some `ui/*` for the design-sync harness — check before deleting those two) | Delete after design-sync check |
| `ModelDownloadProgress.tsx` | 129 | `shared/DownloadProgressToast.tsx` | Delete now |
| `ConsoleToggle.tsx` | 91 | (see broken console feature) | Delete with console_utils decision |
| `SettingTabs.tsx`, `CustomDialog.tsx`, `MessageToast.tsx`, `Logo.tsx`, `ConfirmationModel/` (upstream typo'd dir), `MainNav/`, `BasicBlockNoteTest.tsx` | ~250 | sonner / ui/dialog / Sidebar | Delete now |
| `hooks/useNavigation.ts`, `hooks/meeting-details/useModelConfiguration.ts` | 178 | inline `router.push`; other meeting-details hooks live | Delete now |
| **Verify-first tier:** `ComplianceNotification` (125), `BluetoothPlaybackWarning` (96), `useProcessingProgress` (263), `Calendar/UpcomingMeetings` (231), `DatabaseImport/LegacyDatabaseImport` (231 — meetily-migration leftover; the live path is `HomebrewDatabaseDetector`) | ~950 | may be intended features | Deprecate, confirm intent |

### Dead npm dependencies (zero imports verified)

Delete: all 9 `@remirror/*`, 3 `@tiptap/*` (the editor is BlockNote), `react-markdown` +
`remark-gfm` (only the orphaned AISummary used them), `@heroicons/react` (same), `lodash` +
`@types/lodash`, `@hookform/resolvers`, `@tauri-apps/plugin-updater` + `plugin-process`
(updater removed in specs/0006; neither registered in Rust). The remirror removal likely
also retires the prosemirror `pnpm.overrides` pins (`package.json:98-105`). Verify-first:
`zod` (zero imports today).

Also: `package.json:5` `"main": "electron/main.js"` — no electron dir exists; dead field.

### Config-file duplication (fix, don't just delete)

- `tailwind.config.js` (theme + `tailwindcss-animate`) vs `tailwind.config.ts` (different
  theme + `@tailwindcss/typography`) — **conflicting**; only one loads, so one plugin set is
  silently inactive. Consolidate and verify prose/animation styles in the app.
- `postcss.config.js` (tailwind + autoprefixer) vs `postcss.config.mjs` (tailwind only) —
  same problem; make sure autoprefixer survives.

### Assets
`frontend/public/` Next.js template SVGs (`next.svg`, `vercel.svg`, `file.svg`, `globe.svg`,
`window.svg`) — unreferenced, delete. `src-tauri/icons/`: 10 Windows-Store `Square*Logo.png`
+ `StoreLogo.png` not in the bundle list — delete with the Windows config trim.

---

## Area 6 — Superseded upstream scripts & stale docs

| Item | LOC | Status | Verdict |
|---|---|---|---|
| `frontend/package-app.sh` | 19 | hand-rolled .app packaging, zero references | Delete |
| `frontend/clean_build.sh` | 57 | still allow-listed in `.claude/settings.json:17`; noted as the not-recommended alt in the build skill | Keep (cheap) or delete + update allowlist |
| `frontend/build-dev-app.sh` | 46 | Vinyl-authored ("Dev Vinyl.app" bundle), referenced from `tauri-auto.js` comment | Keep |
| `frontend/scripts/tauri-auto.js`, `auto-detect-gpu.js`, `scripts/shots/*` | ~580 | load-bearing (ADR-0004 dev/prod isolation, build flows, /run-vinyl) | Keep |
| `frontend/dev-gpu.sh`, `build-gpu.sh` | 178+190 | THE build path; dev-gpu.sh intentionally unmodified for upstream mergeability | Keep unmodified |
| `scripts/inject_transcript.py` | 351 | standalone CSV→SQLite dev tool (header still says "Meetily") | Keep as dev aid; rebrand header, or delete if unused |
| `frontend/API.md` | 23 | legacy whisper-server API archive | Delete |
| `frontend/src-tauri/CLEANUP_PLAN.md` | 119 | plans work (audio_v2, `*_old.rs`) **already completed** in `96a8519` | Delete now |
| `frontend/src-tauri/LOGGING_OPTIMIZATIONS.md` | 170 | one-off meetily-era optimization notes | Delete or archive under `docs/` |
| `frontend/src-tauri/NOTIFICATION_TESTING.md` | 85 | manual test scratchpad | Delete or archive |
| `BLUETOOTH_PLAYBACK_NOTICE.md` | 234 | topic still accurate; body says "Meetily"; referenced by nothing | Rebrand + move under `docs/`, or delete |
| `CONTRIBUTING.md` | 141 | 5 "meetily" mentions | Quick rebrand pass |
| `docs/upstream/MEETILY_CLAUDE.md` | 409 | deliberate upstream archive, referenced by CLAUDE.md | Keep |

Housekeeping note: the `upstream` git remote referenced by CLAUDE.md and `/sync-upstream`
is not currently configured (only `origin` exists); `SETUP.md:41-42` documents how to add it.

---

## Area 7 — STT engines & LLM providers: all live (good news)

- **Whisper (2,978 LOC) and Parakeet (2,204 LOC) are both reachable.** Engine selection is
  `transcript_settings.provider` (`audio/transcription/engine.rs:149-216`; default Parakeet,
  `"localWhisper"` validated at `:91`); both have frontend model managers and wired download
  commands. Neither is dead weight — only Whisper's *parallel* command layer is (Area 4).
- **All 7 LLM providers are fully wired end-to-end:** the `LLMProvider` enum
  (`summary/llm_client.rs:87-108`: OpenAI, Claude, Groq, Ollama, OpenRouter, BuiltInAI,
  CustomOpenAI) exactly matches the frontend picker (`ModelSettingsModal.tsx:33`). BuiltInAI
  routes to the `llama-helper` sidecar. The only ghost is **Gemini**: a DB column and
  nothing else (Area 3).

---

## Area 8 — Everything else found along the way

- **Orphaned Rust file:** `whisper_engine/_stderr_suppressor.rs` (82 LOC) — its `mod`
  declaration is commented out (`whisper_engine/mod.rs:8,16`). Not compiled. Delete.
- **Suspect Cargo deps:** `clap` (zero refs; the Cargo.toml comment even admits it's not
  needed as a lib) — delete. `dasp` (zero refs, macOS section) — verify against upstream
  audio merges, then delete. `esaxx-rs` — keep: its `[patch.crates-io]` entry is
  load-bearing for a transitive tokenizer dep.
- **meetily branding, load-bearing (KEEP or migrate deliberately):** DB filename
  `meeting_minutes.sqlite` (`database/manager.rs:81-91`, `data_migration.rs`), recordings dir
  `~/Movies/meetily-recordings` (`audio/recording_preferences.rs:72-101`), Parakeet model
  host `meetily.towardsgeneralintelligence.com` (`parakeet_engine.rs:590`),
  `MEETILY_LLAMA_HELPER` env var. Renaming any of these orphans user data or breaks
  downloads — only do it with an explicit migration.
- **meetily branding, cosmetic (safe, batch into one PR):** `meetily-audio-tap` id
  (`core_audio.rs:143`), `.meetily_decode_` temp prefix (`decoder.rs:294`),
  `meetily_user_id` sessionStorage key (`lib/analytics.ts`), `MeetilyRecoveryDB` IndexedDB
  name (renaming that one is a client-side migration — leave), assorted comments.
- **Tests:** both `frontend/src-tauri/tests/` and the frontend suites target live features
  only; nothing in §5's delete list is covered by a test, so deletions won't break CI.

---

## Suggested cleanup sequencing

Small, reviewable PRs, each independently green through the `/check` gate:

1. **PR 1 — "macOS-only: drop non-mac build scripts & binaries."** All `.bat`/`.ps1`,
   `vs_buildtools.exe`, `sign-windows.ps1` + the `bundle.windows` block, non-mac
   `bundle.targets`, 8 non-mac package.json scripts, `docs/building_in_linux.md`, Windows
   Store icons, template SVGs. ~1.6k LOC + 4.5 MB, zero runtime risk. *Leave all Rust
   `#[cfg]` code alone.*
2. **PR 2 — "docs: retire meetily-era docs + backend stragglers."** Delete `API.md`,
   `CLEANUP_PLAN.md`, `LOGGING_OPTIMIZATIONS.md`, `NOTIFICATION_TESTING.md`; rebrand
   `CONTRIBUTING.md` + `BLUETOOTH_PLAYBACK_NOTICE.md`; delete `config/backend_config.json`,
   the SidebarProvider 5167/8178 defaults, the CSP entries, the whisper_engine backend path
   fallback, fix the `import.rs` jfk.wav test paths, drop the `auth_token` params
   (closes the specs/0028:295 item).
3. **PR 3 — "frontend: remove orphaned components & deps."** The §5 delete-now list +
   16 npm packages + prosemirror overrides + dead `"main"` field. Run the app and the smoke
   path; nothing here is imported, but this is the biggest single diff.
4. **PR 4 — "fix: consolidate tailwind/postcss configs."** Deliberately separate from PR 3
   because it can *change rendering* — verify typography/animation/autoprefixer output.
5. **PR 5 — "rust: delete parallel-processing subsystem + dead commands."** The 11 parallel
   commands + 3 files + managed state, `_stderr_suppressor.rs`, the
   `start_recording_with_devices` wrapper, 2 dead analytics commands, `clap`. Decide the
   console feature here too: fix `--process meetily` or delete
   `console_utils` + its 3 commands + `ConsoleToggle.tsx`.
6. **PR 6 (later, judgment calls) —** deprecate-tier commands (notifications/system-audio
   wrappers, device-reconnection), verify-first frontend orphans
   (`LegacyDatabaseImport`, `UpcomingMeetings`, `BluetoothPlaybackWarning`,
   `useProcessingProgress`, `zod`), `transcript_chunks` write path, `dasp`. Each needs a
   quick "was this an unfinished feature?" check with the roadmap before deleting.

**Explicit non-goals (keep for upstream mergeability or user data):** Rust `#[cfg]`
platform seams incl. `windows_loopback.rs`, target-cfg Cargo dep blocks, `dev-gpu.sh` /
`build-gpu.sh` unmodified, `docs/upstream/MEETILY_CLAUDE.md`, `enhanced_*` and
`geminiApiKey` columns (dormant but harmless), and every meetily-named *data path*
(`meeting_minutes.sqlite`, `~/Movies/meetily-recordings`, model host URL).
