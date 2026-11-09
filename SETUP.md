# Setting up Nixon on a new machine

How to get this repo building and running on a fresh macOS (Apple Silicon) machine.
The product is a Tauri 2 + Next.js 14 desktop app with a Rust core (`frontend/src-tauri/`).

> **Just want to run the app (not develop it)?** Skip the build — install a prebuilt
> release instead. See [Installing a release build](#installing-a-release-build) below.

## 1. Prerequisites

```bash
# Apple toolchain (provides clang, codesign, etc.)
xcode-select --install

# Homebrew
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Rust toolchain (cargo on PATH via rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Node + pnpm
brew install node pnpm

# cmake — required to build the llama-helper sidecar (llama-cpp-sys-2 invokes it
# to compile llama.cpp). Without it the build fails with "is `cmake` not installed?".
brew install cmake
```

## 2. Clone

```bash
git clone git@github.com:sxates/nixon.git
cd nixon
```

If SSH isn't set up on the new machine yet, either add an SSH key to GitHub
(https://github.com/settings/keys) or clone over HTTPS:
`git clone https://github.com/sxates/nixon.git`.

The `upstream` remote (meetily) is configured by `/sync-upstream`; add it if needed:
`git remote add upstream https://github.com/Zackriya-Solutions/meetily.git`.

## 3. Build & run

```bash
cd frontend
pnpm install
./dev-nixon.sh        # DEV launcher — runs as "Dev Nixon" (ai.vinyl.app.debug), isolated data
```

For a production build / install:

```bash
./build-gpu.sh        # production build (also builds the sidecar)
./upgrade-nixon.sh    # rebuild + reinstall /Applications/Nixon.app, preserving data
```

## 4. Gotchas (read these — they will bite you otherwise)

- **The `llama-helper` sidecar is NOT auto-built.** A bare `cargo build`, `pnpm run
  tauri:dev`, or `clean_run.sh` on a clean tree fails with
  `resource path 'binaries/llama-helper-...' doesn't exist`. Only `dev-nixon.sh` /
  `build-gpu.sh` build it (via `cargo build` in `llama-helper/`, then copy to
  `frontend/src-tauri/binaries/`). Run one of those **first**; afterward the simpler paths work.
- **Don't run `dev-gpu.sh` / `build-gpu.sh` directly unless cargo is already on PATH.**
  They're upstream `#!/bin/bash` scripts that don't source rustup. Use `./dev-nixon.sh`,
  which sources `~/.cargo/env` first.
- **Permissions reset per machine.** First launch prompts fresh for **microphone** and
  **audio capture**. Grant both — audio capture is what enables the Core Audio tap to
  capture system/Zoom audio. (System audio does NOT need BlackHole.)
- **Use the bundled `Nixon.app` for reliable system-audio capture.** The bare dev binary
  (`target/debug/nixon`) is ad-hoc signed; its signature changes every rebuild, so macOS
  keeps orphaning its audio-capture grant and the tap silently returns silence (mic still
  works). See ADR-0004.
- **First `pnpm install` leaves a native build unapproved.** pnpm 11 ignores
  `unrs-resolver`'s build script and writes an untracked `frontend/pnpm-workspace.yaml`
  stub (`allowBuilds:\n  unrs-resolver: set this to true or false`). Until you replace
  that placeholder with a real boolean, **every** `pnpm install` exits 1 — including
  Tauri's `beforeDevCommand`, so `dev-nixon.sh` dies right after the sidecar builds. Fix:
  set it to `unrs-resolver: true` (approves the native eslint resolver), then re-run.
  `pnpm approve-builds` does the same thing interactively.
- **Models re-download on first use** (Whisper / Parakeet). Nothing to copy.
- **ffmpeg** is auto-downloaded by `build.rs`. No manual step.

## 5. What does NOT come over with git (intentional)

These are regenerated, not committed:

- `target/` — Rust build artifacts (can be tens of GB; safe to leave behind)
- `frontend/node_modules/` — restored by `pnpm install`
- `frontend/src-tauri/binaries/` — sidecar binaries built by the build scripts
- STT models (`**/models`) — downloaded on demand

## 6. Your meeting data (only if migrating an existing install)

App data lives **outside** the repo, keyed by bundle identifier, and is NOT in git:

- Production: `~/Library/Application Support/ai.vinyl.app/` (DB + settings)
- Dev:        `~/Library/Application Support/ai.vinyl.app.debug/`
- Recordings: `~/Movies/nixon-recordings/` on a fresh install. An install that has
  `~/Movies/meetily-recordings/` keeps writing there for as long as that folder exists —
  the app prefers the legacy folder whenever it is present, so nothing moves.

To carry real meetings to a new machine, copy those directories manually (e.g. via
external drive or `rsync`). A fresh checkout starts with an empty Nixon, which is usually
what you want for development.

## 7. Claude Code config

Repo-scoped Claude config (agents, slash commands, project `settings.json`) lives in
`.claude/` and **travels with git** — nothing to do.

Machine-local Claude state does NOT come with the repo and must be re-established per machine:

- `~/.claude/settings.json` and `~/.claude.json` — global settings, MCP servers, auth (re-login)
- `~/.claude/.mcp.json` — MCP server registrations (figma, paper run on localhost ports)
- Project memory at
  `~/.claude/projects/<project-slug>/memory/` —
  the persistent notes Claude keeps about this project. Copy this dir if you want that
  context to follow you (not required for the app to build/run).

## Installing a release build

For running Nixon on another Mac without setting up the whole dev toolchain. Releases are
published by `./release.sh` to Cloudflare R2 (the download page at https://nixonapp.com/download/)
and, by default, to GitHub (the DMG is attached as a release asset). The easiest way to install is the
download page; the `gh` route below needs GitHub access to the repo.

**Prereqs on the target Mac:** [`gh`](https://cli.github.com) installed and authenticated
(`brew install gh && gh auth login`). The repo is **private**, so download requires auth.

```bash
# list available releases
gh release list -R sxates/nixon

# download a specific release's DMG — pick the newest tag from the list above,
# or omit the tag entirely for the latest release
gh release download <tag> -R sxates/nixon

# install — the DMG lands in the current directory; drag the .app to /Applications
open Nixon_*_aarch64.dmg
```

Tags are plain `vX.Y.Z` and the DMG asset is `Nixon_X.Y.Z_aarch64.dmg`. Nixon's version
line starts at 0.1.0 in this repository; the pre-rename history (Vinyl 0.2.0 – 1.20.0)
lives in a private archive and is not tracked here.

**Why `gh release download` and not the browser:** the builds are ad-hoc signed, not
Apple-notarized. A browser download tags the DMG with `com.apple.quarantine`, so Gatekeeper
blocks it ("Nixon is damaged"). `gh` (and `curl`) don't set that flag, so the app just opens.
If you ever do hit the quarantine block, clear it with:

```bash
xattr -dr com.apple.quarantine /Applications/Nixon.app
```

**Notes**
- Builds are **Apple Silicon (`aarch64`)** only — an Intel Mac would need a separate build.
- First launch prompts fresh for microphone + audio-capture (see the gotchas above).
- Your meetings/recordings do **not** travel with the app — see §6 to migrate data.

### Cutting a release (on the build Mac)

```bash
./release.sh patch          # bump + changelog + build DMG + tag + R2 (nixonapp.com) + GitHub release
./release.sh patch --no-github    # publish to R2 only, skip the GitHub release
./release.sh minor --no-release   # everything except publishing (R2 and GitHub)
./release.sh 1.0.0 --dry-run      # preview, no changes
```

`release.sh` publishes to **Cloudflare R2** (served at `https://nixonapp.com`, specs/0080)
and, unless `--no-github`, also to GitHub (for installs still on the GitHub updater
endpoint). It needs these in `.env.signing` (see `.env.signing.example`; fails fast in
pre-flight if missing, unless `--no-release`):

    CLOUDFLARE_API_TOKEN="…"      # R2 Object Read & Write on the nixon-releases bucket
    CLOUDFLARE_ACCOUNT_ID="…"
    NIXON_SITE_DIR="/path/to/nixonapp"   # site repo checkout; wrangler is run from its pinned install

Upload order is artifacts, then `updates/current.json`, then `updates/early.json` (only if it
moves forward), then `updates/latest.json` last, so an interrupted release never advertises a
version whose files are missing. It then verifies the live feed, and commits + pushes
`public/release.json` to the site repo. `gh` must be authenticated on the build machine unless
you pass `--no-github` or `--no-release`.

#### Early access, then promote (specs/0086)

To try a version on your own Macs before everyone gets it:

```bash
./release.sh patch --early --dry-run   # preview: plans updates/early.json only
./release.sh patch --early             # full release (bump, changelog, signed build, tag, push),
                                       # advertised ONLY on updates/early.json; GitHub pre-release
./release.sh promote 0.15.1 --dry-run  # read-only preflight + the plan
./release.sh promote 0.15.1            # same signed build to everyone; nothing is rebuilt
```

- `--early` writes only `updates/early.json`. `updates/latest.json`, `updates/current.json` and
  the site's `release.json` are untouched, and the GitHub release is a pre-release, so legacy
  installs on the GitHub endpoint never see it. It cannot be combined with `--skip-build` or
  `--no-release`, and it aborts before the build if the site Worker does not serve
  `/updates/early.json`.
- `promote X.Y.Z` needs no clean tree and makes no commit. It needs the Cloudflare credentials,
  `NIXON_SITE_DIR` and `gh`, but no Apple or updater signing keys. It refuses (listing every
  reason) unless tag `vX.Y.Z` exists, the DMG, tarball and `.sig` are on R2, X.Y.Z is newer than
  the live `updates/latest.json`, the `.sig`'s key id matches `plugins.updater.pubkey`, and,
  unless `--no-github`, the GitHub release is a published pre-release whose `.sig` and tarball
  match R2. With `minisign` on PATH it also verifies the tarball cryptographically. It then
  writes `updates/current.json`, then `updates/latest.json` last, verifies, pushes
  `release.json`, uploads the GitHub `latest.json` and flips the pre-release to the latest
  release. Until `latest.json` is written, re-running a failed promote is safe.
- Notes: each feed gets every changelog section newer than the version it advertised, newest
  first, `### Internal` stripped. If 0.15.0 went early and only 0.15.1 is promoted, everyone
  sees both sections. Promote reads the changelog at the tag, never the working tree.
- Turning the switch off on a Mac never downgrades it; it stays put until the everyone track
  passes it.

**Bootstrap.** A Mac only gets the early feed once it runs a build that has the Early access
updates switch (Settings › General › Updates). The first such build reaches a Mac either through
a normal release or by installing the DMG by hand (from nixonapp.com/download, or the GitHub
pre-release's DMG for an early build). After that, turn the switch on and later early releases
arrive through the feed.

## Google Calendar OAuth client (per build machine)

The Google Calendar provider (`specs/0032`, ADR-0010) bakes its OAuth client in at compile
time via `option_env!`, from the **gitignored** `frontend/src-tauri/.env.google`, which both
`release.sh` and `dev-nixon.sh` source. Without it the feature is inert ("not configured" in
Settings, no attendees) — and `release.sh` **aborts** rather than shipping that silently
(pass `--allow-degraded` only if you mean it).

The variables were renamed in the Nixon rebrand (`specs/0057`):

```bash
# frontend/src-tauri/.env.google   (gitignored — create per machine)
NIXON_GOOGLE_CLIENT_ID="<client id>.apps.googleusercontent.com"
NIXON_GOOGLE_CLIENT_SECRET="<client secret>"
```

> **Upgrading an existing machine:** an `.env.google` written before the rebrand still uses
> `VINYL_GOOGLE_CLIENT_ID` / `VINYL_GOOGLE_CLIENT_SECRET`. **Rename the two keys in place**
> (the values are unchanged) — otherwise the next `./release.sh` aborts with
> "Google client id missing". The same applies to any `VINYL_*` overrides you keep in
> `.env.signing` or your shell profile: the prefix is now `NIXON_`.

Credentials come from a personal GCP project, **Desktop app** credential type; the client
secret is non-confidential for desktop apps (PKCE is the protection) but the file stays out
of git anyway.

## Signing & notarization (for internet-distributable builds)

By default the DMG is **ad-hoc signed** — fine for `gh release download` installs, but a
browser download is blocked by Gatekeeper. To ship a DMG that opens from a normal web
download you need Apple **Developer ID signing + notarization**. The plumbing is already
in `release.sh`; it's gated on credentials, so it stays a no-op until you add them. See
**[ADR-0008](docs/decisions/ADR-0008-macos-signing-notarization.md)** for the rationale.

**One-time Apple setup:**
1. Join the **Apple Developer Program** ($99/yr). Note your **Team ID**.
2. Create a **Developer ID Application** certificate (Xcode → Settings → Accounts →
   Manage Certificates → + *Developer ID Application*, or developer.apple.com). Its
   private key must be in your login keychain. Verify:
   `security find-identity -v -p codesigning` → should list `Developer ID Application: … (TEAMID)`.
3. Create notarization creds — an **App Store Connect API key** (recommended: App Store
   Connect → Users and Access → Integrations; download the `.p8` once, note Key ID +
   Issuer ID), or an **app-specific password** for your Apple ID.

**Per build machine:**
```bash
cp .env.signing.example .env.signing   # gitignored
# fill in APPLE_SIGNING_IDENTITY + one notarization block, then:
./release.sh patch                     # now signs + notarizes + staples the DMG
```
With `.env.signing` present, `release.sh` loads it, fails fast if the identity isn't in
the keychain, and after the build runs `spctl` + `xcrun stapler validate` to confirm.
Verify by hand with `spctl -a -vvv -t install Nixon.app` (want `source=Notarized Developer ID`).

**Public hosting:** the repository and its releases are public, so a notarized DMG
downloads and opens from the release page directly.

### In-app updates (specs/0058)

Installed apps poll `https://nixonapp.com/updates/latest.json` (R2; older installs poll the
latest GitHub release's `latest.json`) and install the signed `Nixon.app.tar.gz` it points to. `release.sh` produces and uploads both; it refuses to
publish without the update signing key.

**One-time:** `cd frontend && pnpm tauri signer generate -w ~/.tauri/nixon-updater.key`,
commit the printed **public** key into `frontend/src-tauri/tauri.conf.json`
(`plugins.updater.pubkey`), and add to `.env.signing`:

    TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/nixon-updater.key)"
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="…"

**Back the private key up outside the repo.** If it is lost, every installed copy will
reject updates signed with a replacement key and users must reinstall by hand.
Dev builds (`dev-nixon.sh`) never check for updates (`NIXON_DISABLE_UPDATER=1`).
