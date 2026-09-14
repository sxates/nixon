# 0002 — Rebrand to Vinyl (APP_NAME)

- **Status:** Implemented (2026-06-24)
- **Owner agent(s):** rust-core-engineer (+ frontend-engineer)
- **Roadmap phase:** Phase 1

## Context / Problem
The fork still identifies as "Meetily" — window title, bundle identifier, and the macOS
app-data directory. We want it to be "Vinyl" (a name that may still change, so keep it a
single configurable source of truth).

**Update (2026-06-23): the bundle-identity slice was pulled forward.** Because the user runs
the installed meetily alongside our dev build, and Tauri derives the app-data dir + the
`tauri-plugin-single-instance` lock + TCC permissions from the bundle identifier, sharing
`com.meetily.ai` meant our dev build read/wrote the user's **real** `meeting_minutes.sqlite`
and couldn't run concurrently. So we already changed `tauri.conf.json` →
`identifier: com.vinyl.dev`, `productName: Vinyl`. That isolates the SQLite DB, settings,
analytics, and onboarding (all identifier-derived) into
`~/Library/Application Support/com.vinyl.dev/`.

**Remaining isolation leak (do here):** a few paths bypass the identifier via a hardcoded
`"Meetily"` literal:
- `summary/templates/loader.rs:27` — custom templates resolve to
  `~/Library/Application Support/Meetily/templates/` in **dev and prod** (identifier-independent).
- Production fallbacks in `whisper_engine/whisper_engine.rs:122`,
  `parakeet_engine/parakeet_engine.rs:142`, `summary/summary_engine/model_manager.rs:148`
  (`.join("Meetily")` — dormant in dev, which uses an in-repo `models/` dir).
Replace these with an identifier-/`APP_NAME`-derived path (single source of truth), not a new
literal.

## Goals
- Product name, window title, and bundle identifier reflect `APP_NAME` (currently "Vinyl").
- `APP_NAME` is changeable from as few places as possible.
- Existing user data (DB + models) is preserved across the app-data-dir change.

## Non-goals
- New icons/branding art (separate polish task). Windows/Linux packaging parity.

## Approach
Change the Tauri identity in `frontend/src-tauri/tauri.conf.json` (`productName`,
`identifier`) and `package.json`. Add a **one-time data migration** that, on first launch,
detects the old `Meetily/` app-data dir and moves/copies `meeting_minutes.sqlite` (+ `-wal`,
`-shm`) and `models/` into the new `Vinyl/` dir before opening the DB.

## Design
### Data model
No schema change. Migration is filesystem-level, run before `database/manager.rs` opens the
pool — likely in app setup (Tauri `setup` hook) guarded by a "migrated" marker.

### Tauri IPC
None expected.

### UI
Window title / about strings; any hardcoded "Meetily" in `frontend/src/`.

## Tasks
1. [x] Update `tauri.conf.json` (`productName: Vinyl`, `identifier`) — first `com.vinyl.dev` for
   isolation from installed meetily, then `ai.vinyl.app` at full rebrand (dev `ai.vinyl.app.debug`).
2. [x] Fix the hardcoded `"Meetily"` path leak — all storage (templates, Zoom/notification
   settings, model fallbacks) now derives from the identifier-rooted `src/app_paths.rs` single
   source of truth (also fixed the 0008 lowercase `meetily/` settings leak).
3. [x] Rename the crate/npm package `meetily` → `vinyl` (`Cargo.toml`, `package.json`; lib stays
   `app_lib`, so the binary becomes `vinyl`). User-visible name strings were already handled in
   `specs/0006`. Kept functional names (`~/Movies/meetily-recordings/`, Parakeet model URL).
4. [x] **Decided (user, 2026-06-24):** production identifier → `ai.vinyl.app`
   (dev `ai.vinyl.app.debug`).
5. [x] App-data-dir migration: implemented in `src/data_migration.rs` — a one-time, marker-guarded,
   **non-destructive** first-launch import from the old identifier dir (`com.vinyl.dev` →
   `ai.vinyl.app`; DB + settings copied, models moved). Verified end-to-end on the dev identifier
   (imported 7 files + models; 11 meetings loaded).

## Acceptance criteria
- Fresh install creates the new app-data dir; an existing Meetily install migrates DB +
  models with no data loss.
- App window/about shows "Vinyl"; `/check` passes; smoke path still works.

## Risks / open questions
- **TODO:** final bundle identifier. Renaming the app-data dir mid-stream risks data loss if
  migration is wrong — test with a copy of a populated `Meetily/` dir first.

## Verification
Populate old `Meetily/` dir, launch, confirm meetings/models present; run the smoke path.
