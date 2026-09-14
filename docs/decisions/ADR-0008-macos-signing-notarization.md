# ADR-0008: macOS code signing & notarization for distribution

Status: Accepted
Date: 2026-06-27

## Context

Release DMGs are currently **ad-hoc signed** (`bundle.macOS.signingIdentity = "-"`
in `frontend/src-tauri/tauri.conf.json`). Ad-hoc builds:

- carry no Team ID, so macOS Gatekeeper blocks them on any machine other than the
  one that built them ("Vinyl is damaged / unidentified developer") **if** the file
  is quarantined;
- are usable today only because we distribute via `gh release download` (and `curl`),
  which don't set `com.apple.quarantine`. A normal browser download is blocked.

This is fine for a private, invite-only TestFlight-style hand-off, but it does **not**
allow open internet distribution (a download link on a website or a public release).

For a build that opens from any browser download, Apple requires:
1. signing with a **Developer ID Application** certificate (paid Apple Developer
   Program; distinct from the "Apple Development" cert), under the **hardened runtime**;
2. **notarization** — uploading the signed app to Apple's notary service, which scans
   it and issues a ticket; and
3. **stapling** the ticket to the `.app`/`.dmg` so Gatekeeper passes it offline.

## Decision

Add an **env-gated** signing path to `release.sh` that is a no-op until credentials
exist, so nothing changes for contributors without a Developer ID:

- `release.sh` sources a **gitignored `.env.signing`** (template: `.env.signing.example`)
  before the build. Tauri's bundler already reads `APPLE_SIGNING_IDENTITY`,
  `APPLE_API_KEY`/`APPLE_API_ISSUER`/`APPLE_API_KEY_PATH` (or `APPLE_ID`/
  `APPLE_PASSWORD`/`APPLE_TEAM_ID`) from the environment, so when those are present
  `tauri build` signs with Developer ID, notarizes, and staples automatically.
- When `.env.signing` is absent (or `APPLE_SIGNING_IDENTITY` is unset), the build
  **falls back to ad-hoc** exactly as before. `signingIdentity` stays `"-"` in
  `tauri.conf.json` (the env var overrides it when set) so the default checkout still
  builds with no Apple account.
- `release.sh` fails fast if `APPLE_SIGNING_IDENTITY` is set but the cert isn't in the
  keychain (before the long build), and verifies `spctl`/`stapler` after a Developer
  ID build.
- Tauri notarizes + staples the `.app` but ships it inside an **un-notarized DMG**.
  `release.sh` therefore submits the finished DMG to `notarytool` and `stapler staple`s
  the container too, so a *browser* download (which sets `com.apple.quarantine`) passes
  Gatekeeper offline rather than needing a first-open online check.
- `hardenedRuntime` is already `true` and `entitlements.plist` already sets
  `com.apple.security.cs.disable-library-validation` (needed so the bundled,
  ad-hoc-signed sherpa-onnx / onnxruntime dylibs load — see ADR-0005). Disabling
  library validation is compatible with notarization.

Credentials live only in `.env.signing` (or the keychain) and are **never committed**.

## Distribution note

`sxates/nixon` is private, and **release assets on a private repo require auth to
download** — a notarized DMG alone isn't publicly reachable. Open internet distribution
additionally requires either making the repo/releases public, or hosting the notarized
DMG on a public URL/CDN. Notarization removes the Gatekeeper block; public hosting
removes the auth wall. Both are needed.

## Consequences

- Once a Developer ID cert + notarization creds are in place, `./release.sh` produces a
  notarized, stapled DMG with no code changes — just drop in `.env.signing`.
- Each Developer ID build adds ~2–10 min for Apple's notary scan.
- Until then, releases remain ad-hoc and must be installed via `gh release download`.
- Auto-update (Tauri updater: `TAURI_SIGNING_PRIVATE_KEY` + a `latest.json` endpoint)
  is a separate signing system, intentionally out of scope here.
