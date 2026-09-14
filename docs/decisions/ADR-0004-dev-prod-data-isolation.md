# ADR-0004 — Separate dev and production data via distinct bundle identifiers

- **Status:** Accepted
- **Date:** 2026-06-24

> **Update (2026-06-24, `specs/0002`):** the identifiers below were renamed by the full rebrand —
> production `com.vinyl.dev` → **`ai.vinyl.app`**, dev `com.vinyl.dev.debug` → **`ai.vinyl.app.debug`**.
> The *decision* (isolate dev/prod via distinct identifiers) stands unchanged; only the strings
> differ. A non-destructive first-launch migration (`src/data_migration.rs`) moved existing data to
> the new dirs. The original identifiers are kept verbatim below as the historical record.

## Context
Vinyl is now used for two things at once: a daily-driver **production** app that captures real
meetings, and an actively-rebuilt **dev** build for development/testing. Tauri keys an app's
data directory (`~/Library/Application Support/<identifier>/` — SQLite DB, settings, models),
its single-instance lock, and its TCC permissions to the **bundle identifier**. With both builds
sharing `com.vinyl.dev`, running the dev build read/wrote the user's real `meeting_minutes.sqlite`
— an experimental migration or test could corrupt or pollute production meetings.

Separately, the dev build is the wrong tool for reliable capture: it runs as a bare, ad-hoc-signed
binary (`target/debug/meetily`) whose signature changes every rebuild, so macOS keeps orphaning its
Screen Recording grant (no `.app`/bundle id for macOS to register or for `tccutil` to reset). The
Core Audio system-audio tap silently returns silence without that permission.

## Decision
- **Production** = the bundled **`Vinyl.app`** (built via `frontend/build-gpu.sh`), keeping the
  existing identifier **`com.vinyl.dev`** — so the user's current data is preserved with zero
  migration. Installed to `/Applications`; upgraded in place via `frontend/upgrade-vinyl.sh`
  (backs up the DB, rebuilds, swaps the app). Data lives outside the `.app`, so upgrades never
  lose it; DB migrations are forward-only/additive.
- **Dev** = `tauri dev` runs under a distinct identifier **`com.vinyl.dev.debug`** and
  productName/title **"Dev Vinyl"**, via `frontend/src-tauri/tauri.dev.conf.json` merged with
  `--config` in `frontend/scripts/tauri-auto.js` (dev command only; the production `build` is
  untouched). Its data is the separate `~/Library/Application Support/com.vinyl.dev.debug/`.
- The dev build also shows an in-UI **"DEV" badge** (gated on `process.env.NODE_ENV`) so the two
  are distinguishable when both run at once (they can — separate single-instance locks).

> Note: "production" keeping the `…dev` suffix is a deliberate, temporary naming oddity to avoid
> migrating live data. The clean split (production `com.vinyl`, dev `com.vinyl.dev`) with a
> data-migration belongs to the rebrand in `specs/0002` / ADR-0002.

## Consequences
- Dev testing can no longer touch production meeting data; both apps can run simultaneously.
- The dev build gets its own (initially empty) DB and models dir — it re-onboards and
  re-downloads models on first launch (one-time, isolated).
- Production Screen Recording is granted once to `Vinyl.app` and persists across launches; an
  in-place upgrade (new ad-hoc signature) may require a one-click re-grant. A real signing
  identity would remove even that — deferred until wider distribution.
- `tauri.dev.conf.json` carries the full window block (not just `title`) because Tauri's
  `--config` replaces arrays rather than deep-merging them by index.

## Dev-build gotchas

- **Ad-hoc signature churn — RESOLVED (2026-07-03):** the "signature changes every rebuild"
  problem described in Context grew a second symptom once API keys moved to the Keychain
  (ADR-0009): every rebuild invalidated the Keychain item ACLs for `ai.vinyl.app.debug`, so
  the dev app triggered a login-keychain **password prompt** on each key read. Fix: `tauri dev`
  now builds through a cargo wrapper (`src-tauri/scripts/cargo-dev-sign.sh`, set as
  `build.runner` in `tauri.dev.conf.json` only) that re-signs the debug binary after every
  build with a stable **Apple Development** identity (`$NIXON_DEV_SIGNING_IDENTITY` override, renamed in specs/0057;
  auto-detected from the keychain; silently no-ops back to ad-hoc when no identity exists,
  e.g. CI or a contributor without an Apple cert). Same designated requirement on every
  rebuild ⇒ one "Always Allow" (Keychain) and one Screen Recording grant now persist. The
  binary is signed with `get-task-allow` (`scripts/dev-entitlements.plist`) so lldb attach
  still works. Expect **one final** password/permission round the first time a newly-signed
  build touches items created under an old ad-hoc signature. Release builds are untouched
  (the bundler signs those with the production identity, ADR-0008).

- **Stale notification icon after a rebrand/icon change** (2026-07-01, `specs/0029` WS6.3;
  re-diagnosed 2026-07-08, `specs/0041` WS8): a macOS notification wears the icon that
  **Notification Center has cached for the sending bundle id** — not the icon inside the
  installed `.app`. Early Vinyl builds shipped under `ai.vinyl.app` while `icons/` still held
  Meetily artwork, seeding that per-bundle-id cache; rebuilding or reinstalling a
  correctly-iconed bundle **never evicts it** (multiple production reinstalls confirmed this —
  the earlier "rebuild + `lsregister -kill -r`" advice that lived here is superseded). The
  Rust side can't override it either: `tauri-plugin-notification` delegates to `notify-rust`,
  which documents that macOS ignores per-notification icons, so the cached bundle icon is
  authoritative (comment at the builder in `src/notifications/system.rs`). Fix: run
  **`scripts/reset-notification-icon-cache.sh`** once against the affected install — it
  force-re-registers `/Applications/Vinyl.app` with LaunchServices (`lsregister -f`) and
  restarts the Notification Center processes (`NotificationCenter`, `usernoted`; absent
  process names are skipped gracefully) so they drop the cached attribution. Verify by
  triggering a recording-started notification; if the old icon persists, log out/in or
  reboot rebuilds the per-session caches.
