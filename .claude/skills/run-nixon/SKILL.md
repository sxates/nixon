---
name: run-nixon
description: Build, run, drive, and screenshot the Nixon desktop app (Tauri + Next.js). Use when asked to run/start/launch Nixon, screenshot a Nixon page/route, drive its UI, or verify a frontend change in the running app.
---

# Run Nixon

Nixon is a **Tauri 2 + Next.js 14** macOS desktop app (record → transcribe → summarize
meetings, on-device). Paths below are relative to the repo root.

There are two surfaces, and they're driven very differently:

- **The frontend (React UI) — where almost all PRs land.** Drive it in real Chrome via
  the committed driver **`.claude/skills/run-nixon/driver.mjs`** (puppeteer-core +
  system Chrome). It injects a Tauri `invoke` shim so pages render and screenshots, and
  can type into the UI. **This is the agent path — start here.**
- **The native Tauri window** (system-audio capture, diarization, real calendar). Needs
  macOS mic + screen-recording permission, Metal, and downloaded models — **not
  automatable headless.** Human path only (`./dev-nixon.sh`), see below.

## Prerequisites
- macOS with **Google Chrome** at `/Applications/Google Chrome.app` (override with
  `CHROME_PATH`). The driver uses `puppeteer-core` (a `frontend` devDep — no Chromium
  download).
- Node 18+ and pnpm 11.

## Setup
```bash
cd frontend
pnpm install              # if it errors ERR_PNPM_IGNORED_BUILDS, approve esbuild:
                          #   the gitignored frontend/pnpm-workspace.yaml must list
                          #   esbuild + unrs-resolver under onlyBuiltDependencies/allowBuilds,
                          #   then: pnpm rebuild esbuild
```
`puppeteer-core` is already in `frontend/package.json` devDependencies. If missing:
`pnpm add -D puppeteer-core` (run in `frontend/`).

## Run (agent path) — drive the frontend

**1. Start the dev server** (from `frontend/`). The app's default port 3118 is often
already taken; use an isolated port so you're driving *your* code:
```bash
cd frontend && pnpm exec next dev -p 3119
```
Wait for `✓ Compiled` / a `200` on `http://localhost:3119/`.

**2. Drive + screenshot** with the driver (from the repo root, server left running):
```bash
# A page (renders through the Tauri invoke shim + onboarding bypass):
node .claude/skills/run-nixon/driver.mjs --route /people --out /tmp/people.png

# An interaction — type into a selector, then screenshot (drives People search):
node .claude/skills/run-nixon/driver.mjs --route /people \
  --type jordan --selector 'input[aria-label="Search people"]' --out /tmp/people-search.png

# The home dashboard:
node .claude/skills/run-nixon/driver.mjs --route / --out /tmp/home.png
```
The driver prints the screenshot path, the first ~600 chars of visible text, and any
page/console errors. **Look at the PNG** (Read it). Options: `--port` (3119),
`--delay` ms after load (2500), `--type`/`--selector` to interact, `--width`/`--height`.

**Extending the mocks:** the driver stubs the Tauri commands the shell + common pages
call (in the `MOCKS` object). Driving a deeper page that calls a new command? The driver
prints the resulting error (e.g. a null where a list/`.is_recording` is expected) — add
that command to `MOCKS` with a sane shape and re-run. That iterate-on-errors loop is the
intended workflow.

## Run (human path) — the real native app
```bash
./dev-nixon.sh            # builds (cargo + llama-helper sidecar) and opens "Dev Nixon"
```
Opens a native window under identifier `ai.vinyl.app.debug` (isolated data). Needed for
anything touching **real** system-audio capture, live diarization, or EventKit calendar —
none of which the browser path exercises. Requires granting mic + screen-recording
permission. Not usable headless and not scriptable from this harness; screenshot the
window with macOS `screencapture` if you need native evidence.

## Gotchas (battle scars)
- **`chrome --headless --screenshot` hangs on the dev server.** Next dev keeps an HMR
  websocket open, so Chrome's "wait for network idle" never fires and `--screenshot` /
  `--dump-dom` never return. The driver uses puppeteer with `waitUntil:'domcontentloaded'`
  + a fixed delay to sidestep this. Don't go back to raw `chrome --screenshot` here.
- **The frontend can't render in a plain browser without the Tauri shim.** Every route is
  under `app/layout.tsx`, which mounts providers that call `invoke` on bootstrap. Without
  the shim, `RecordingStateProvider` throws `Cannot read properties of null (reading
  'is_recording')` and the whole tree falls into the error boundary. The driver's
  `__TAURI_INTERNALS__.invoke` shim is mandatory.
- **Onboarding gate.** `app/layout.tsx` shows the "Welcome to Nixon" flow unless
  `invoke('get_onboarding_status')` returns `{ completed: true }`. The driver mocks that —
  without it you screenshot the welcome screen, not the app.
- **`get_recording_state` must be a full object** (`{ is_recording:false, ... }`), not
  `null` — it's polled every 500ms and `.is_recording` is read unguarded.
- **`macos say`-based audio tests and the native window are out of scope here** — this
  harness is the React UI only.
- **`DEV` badge** in screenshots is expected in the dev build (it's gated to dev).

## Troubleshooting
- `ERR_PNPM_IGNORED_BUILDS: esbuild` on `pnpm install`/`pnpm test` → approve esbuild in
  `frontend/pnpm-workspace.yaml` (gitignored), then `pnpm rebuild esbuild`.
- Driver: `Cannot read properties of null (reading 'X')` in a provider → a mocked command
  returned `null` where an object/array was expected. Add it to `MOCKS` with the right shape.
- Screenshot shows the welcome/onboarding screen → `get_onboarding_status` mock missing or
  not `{ completed: true }`.
- `EADDRINUSE :3118` → something already runs on the default port; use `-p 3119` (the
  driver defaults to 3119).
- Driver can't find `puppeteer-core` → run `pnpm add -D puppeteer-core` in `frontend/`
  (the driver resolves it from `frontend/node_modules`).
