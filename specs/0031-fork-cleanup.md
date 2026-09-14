# Spec 0031 — Fork Cleanup (second ring)

**Status:** ✅ Implemented (2026-07-02) — all gates green incl. Next production build.
Deviations from the audit, found during re-verification: the old `AISummary` cluster is
KEPT (it is the legacy-format fallback inside `BlockNoteSummaryView.tsx:228` — audit error;
dropping it needs a product decision first), `@tiptap/pm` KEPT (load-bearing prosemirror
aliases in `next.config.js`), `@heroicons/react` KEPT (used by the retained AISummary),
`ui/sheet|input-group|progress` KEPT (design-sync re-exports), `track_meeting_ended`
demoted to an internal fn rather than deleted (live Rust caller on recording stop).
**Branch:** `chore/0031-fork-cleanup`
**Source of truth:** `docs/audits/2026-07-01-fork-deprecation-audit.md` — this spec executes
its "Suggested cleanup sequencing" PRs 1–5. Every deletion below was verified zero-caller in
the audit; re-verify before deleting anything the audit marked "verify-first" (those are NOT
in scope here — they live in the audit's PR 6 tier and the roadmap's engineering-health track).

## Scope (audit PRs 1–5, sliced by file ownership)

### WS-A — macOS-only: scripts, binaries, config, docs (audit PR 1 + PR 2 doc/config half)
- Delete all Windows/Linux build scripts (9 `.bat`/`.ps1`), `frontend/vs_buildtools.exe`
  (4.5 MB), `frontend/scripts/load-env.ps1`, `src-tauri/scripts/sign-windows.ps1`,
  `docs/building_in_linux.md`, Windows-Store icons, Next.js template SVGs in
  `frontend/public/`, `frontend/package-app.sh`.
- `tauri.conf.json`: `bundle.targets` → `["app","dmg"]`; drop the `bundle.windows` block;
  tighten CSP `connect-src` (drop `localhost:5167`/`:8178`, keep `11434` Ollama).
- Delete meetily-era docs: `frontend/API.md`, `src-tauri/CLEANUP_PLAN.md`,
  `src-tauri/LOGGING_OPTIMIZATIONS.md`, `src-tauri/NOTIFICATION_TESTING.md`;
  delete `src-tauri/config/backend_config.json`; rebrand `CONTRIBUTING.md`; rebrand + move
  `BLUETOOTH_PLAYBACK_NOTICE.md` → `docs/`.
- Keep (explicit): Rust `#[cfg]` seams incl. `windows_loopback.rs`, `dev-gpu.sh`/`build-gpu.sh`
  unmodified, `clean_build.sh`, `docs/upstream/MEETILY_CLAUDE.md`, all meetily-named data paths.

### WS-B — Frontend: orphaned surface + deps + config duel (audit PR 3 + PR 4)
- Delete the zero-importer clusters: old `AISummary/` view (keep `BlockNoteSummaryView.tsx`),
  `TranscriptView.tsx`, `AudioPlayer.tsx` + `useAudioPlayer.ts`,
  `molecules/form-components/`, `ModelDownloadProgress.tsx`, `ConsoleToggle.tsx`,
  `SettingTabs.tsx`, `CustomDialog.tsx`, `MessageToast.tsx`, `Logo.tsx`,
  `ConfirmationModel/`, `MainNav/`, `BasicBlockNoteTest.tsx`, `hooks/useNavigation.ts`,
  `hooks/meeting-details/useModelConfiguration.ts`; `ui/sheet|input-group|progress` only
  after checking `.ds-entry.tsx` re-exports.
- `package.json`: remove 16 dead deps (9 `@remirror/*`, 3 `@tiptap/*`, `react-markdown`,
  `remark-gfm`, `@heroicons/react`, `lodash` + types, `@hookform/resolvers`,
  `@tauri-apps/plugin-updater` + `plugin-process`), prosemirror `pnpm.overrides`, dead
  `"main": "electron/main.js"`, the 8 non-mac `tauri:*` script variants.
- Consolidate `tailwind.config.js` vs `.ts` and `postcss.config.js` vs `.mjs` into ONE of
  each carrying the UNION of plugins (typography + animate; autoprefixer kept) — this is the
  rendering-risk item; verify with a production `next build` and visual spot-check.
- `SidebarProvider.tsx`: drop the dead `localhost:5167`/`8178` server defaults.

### WS-C — Rust: dead subsystems + commands + stragglers (audit PR 5 + PR 2 Rust half)
- Delete the parallel-Whisper subsystem (~1,039 LOC: `parallel_commands.rs`,
  `parallel_processor.rs`, `system_monitor.rs`, 11 commands, `ParallelProcessorState`).
- Delete the console feature (broken `--process meetily`): `console_utils/` module + its 3
  commands (frontend `ConsoleToggle.tsx` deleted in WS-B).
- Delete `whisper_engine/_stderr_suppressor.rs` (mod commented out), the
  `start_recording_with_devices` thin wrapper, the 2 uncalled analytics commands.
- Backend stragglers: drop `auth_token` params in `api/api.rs` (closes specs/0028:295),
  delete the `backend/whisper-server-package` model-path fallback in `whisper_engine.rs`,
  fix/drop the `import.rs` jfk.wav test paths.
- `Cargo.toml`: remove `clap`; trim the stale Windows/Linux build-comment block. Keep `dasp`
  (verify-first tier), keep `esaxx-rs` patch (load-bearing).

## Acceptance
1. Full gates: `cargo check` / `clippy -D warnings` / `cargo test` (metal) and `pnpm lint` /
   `pnpm test` clean; `pnpm build` (Next) succeeds with the consolidated configs.
2. App still builds and launches; record → transcript → summary smoke path noted for the
   next manual pass.
3. No item from the audit's keep-list or verify-first tier is touched.
