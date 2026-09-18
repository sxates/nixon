# Screenshot pipeline (`scripts/shots`, specs/0060)

Three drivers share one manifest (`routes.json`) to produce two PNG sets — see
`docs/screenshots/README.md` at the repo root for what the sets are *for* and the owner
procedure. This file is the implementation reference: the manifest schema, every command's
flags, the URL params the mock understands, and the real-window control protocol.

## The manifest (`routes.json`)
An array of entries. Each is expanded (by `lib/manifest.mjs`) into one shot per theme.

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Unique id; output file is `<name>.<theme>.png`. |
| `route` | yes | App path, must start with `/`. May include its own query string. |
| `themes` | no | Array of `faceplate`\|`deck`; default is both. |
| `viewport` | no | `[width, height]`; default `[1280, 860]`. |
| `wait` | no | Milliseconds to dwell after the ready signal before capturing; default `1500`. `record-live` sets `8000` so the level meters have moved. |
| `sidebar` | no | `"open"` expands the sidebar (collapsed is the default, fresh-profile state). |
| `onboardingStep` | no | `1..5`; forces the onboarding wizard to that step instead of the completed app. |
| `state` | no | Free-form annotation; only `"recording"` has code behind it — `real.mjs` reads it to run the record-live choreography (see below). Other values (e.g. `"downloading"` on `onboarding-3`) are documentation only; that screen's actual downloading state comes from `onboardingStep` alone. |
| `real_only` | no | `true` skips the entry for headless `pnpm shots` (e.g. `record-live` — headless has no audio pipeline to drive it). |
| `ignore` | no | Array of `{x, y, w, h, why}` pixel rects that `pnpm shots:diff` overwrites with the base image before comparing — for regions that are non-deterministic between runs by design (the live now-line, the footer tape counter, spinner icons), not a way to hide a real bug. |

Validated by `lib/manifest.mjs::loadManifest` (duplicate names, missing `route`/leading `/`,
unknown theme, out-of-range `onboardingStep` all throw).

## The three commands

### `pnpm shots` → `run.mjs`
Headless capture of every non-`real_only` manifest entry, in both themes, to
`docs/screenshots/headless/` by default.

```
node scripts/shots/run.mjs [--out DIR] [--only a,b] [--theme deck|faceplate] [--no-build] [--worktree-build]
```
- `--out DIR` — output directory (default `docs/screenshots/headless`).
- `--only a,b` — comma-separated entry names to capture (matches `name`, not the output file).
- `--theme deck|faceplate` — capture just one theme.
- `--no-build` — reuse the existing `frontend/out/` export instead of rebuilding it.
- `--worktree-build` — build the export in an isolated git worktree (see below) instead of
  in place. `run.mjs` also does this automatically when it detects a dev server already
  listening on `:3118`.

Writes `manifest-run.json` alongside the PNGs (git SHA, timestamp, per-shot status) and
exits non-zero if any capture errored.

### `pnpm shots:diff` → `diff.mjs`
Pixelmatches the current `docs/screenshots/headless/*.png` against a git ref and writes a
contact sheet.

```
node scripts/shots/diff.mjs [--dir DIR] [--base REF] [--out DIR] [--strict]
```
- `--dir DIR` — set of PNGs to diff (default `docs/screenshots/headless`).
- `--base REF` — git ref to diff against (default `HEAD`).
- `--out DIR` — where to write the contact sheet (default `docs/screenshots/diff`, gitignored).
- `--strict` — exit `1` if anything changed (`0 changed` on a clean tree is the expected
  result before committing new headless PNGs).

`ignore` rects from `routes.json` are applied per-entry before comparing (see manifest table
above). Prints `N changed / N same / N new → <path>/index.html` and exits accordingly.

### `pnpm shots:real` → `real.mjs`
Drives the running Dev Nixon window over the control protocol (below) and captures it with
macOS `screencapture` into `docs/screenshots/real/` by default. Every manifest entry
(including `real_only: true` ones) is eligible.

```
node scripts/shots/real.mjs [--out DIR] [--only a,b] [--theme deck|faceplate] [--audio PATH] [--port N]
```
- `--out DIR` — output directory (default `docs/screenshots/real`).
- `--only a,b` — comma-separated entry names.
- `--theme deck|faceplate` — capture just one theme.
- `--audio PATH` — audio file played through `afplay` during the `record-live` take
  (default: the first of `~/Movies/{meetily,nixon}-recordings/nixon-demo-01/audio.mp4` that
  exists — i.e. the `--demo` profile's own seeded audio).
- `--port N` — control listener port (default: read from
  `~/Library/Application Support/ai.vinyl.app.debug/dev-control.port`, written by the app on
  startup when the listener is enabled).

Requires `./dev-nixon.sh --demo` (or `--control`) already running — see "Control protocol"
below — plus Screen Recording permission for the terminal process running this script, and
Microphone/Audio Capture permission for the `record-live` shot. On `SIGINT`/`SIGTERM` it
stops any in-progress take and hides the DEV badge again before exiting.

## URL params (headless only)
`tauri-mock.js` reads three query params, set by `run.mjs`'s `expand()`:
- `theme=deck|faceplate` (also accepts `dark|light`) — written to the real theme storage key
  before the app boots, so the app's actual dark/light rendering is exercised (not a CSS
  override).
- `onboardingStep=1..5` — forces the onboarding wizard to that step; omitted for the
  completed-onboarding app.
- `sidebar=open` — expands the sidebar (default is collapsed, matching a fresh profile).

## `shotReady` / `data-shot` hooks
Both drivers need to know the app has actually rendered before capturing, and the real
driver additionally needs to hide dev-only chrome:
- `document.documentElement.dataset.shotReady` is set to `"1"` by `src/app/layout.tsx`
  ~400 ms after mount. `screenshot.mjs`/`cdp.mjs` (headless) poll it directly via CDP
  `Runtime.evaluate`; the control listener's `ready` command polls it indirectly by having
  the app call the `dev_shot_ping` Tauri command in a loop (`eval` has no return value, so
  the round trip is required — see `control.rs`).
- `document.documentElement.dataset.shot = "1"` hides the "DEV" badge
  (`globals.css`: `html[data-shot="1"] [data-dev-badge] { display: none }`). The real
  driver sets this via the `hide_dev_badge` command before every capture (a full
  `navigate` reload clears it, so it's re-sent after any navigation) and clears it again on
  exit.

## Control protocol (real captures only)
`real.mjs` talks NDJSON-over-TCP-loopback to a debug-only listener
(`frontend/src-tauri/src/dev_fixtures/control.rs`) — one JSON object per line in, one
`{"ok":true,...}` or `{"ok":false,"error":...}` reply per line out (`lib/control.mjs` is the
client). The listener only spawns when `NIXON_DEV_CONTROL=1` **and** the running bundle is
the `.debug` identifier (`guard::allowed`, fail-closed) — `./dev-nixon.sh --demo` sets that
env var for you (so does the bare `--control` flag); it never spawns in a release build. Its
port is written to `~/Library/Application Support/ai.vinyl.app.debug/dev-control.port`.

Commands (see `control.rs` for the exact request/reply shapes): `ping`, `navigate {route}`,
`theme {value: light|dark}`, `onboarding_step {value: 1..5}`, `onboarding_complete`,
`resize {w, h}`, `ready {timeout_ms?}`, `hide_dev_badge {value}`, `window` (returns the
macOS window number `screencapture -l` needs), `start_recording {title?}`, `stop_recording`.
`start_recording`/`stop_recording` call the same internal entry points the real UI's
record/stop use, with `stop_recording` best-effort discarding the throwaway take's DB row
and on-disk folder afterwards — `control.rs` documents a known residual (a real meeting
selected in the live window when a take starts could have its cached folder path clobbered),
which is why `real.mjs` navigates to `/` before every `start_recording`.

## The isolated worktree build
`next build` and `next dev` both write to `.next`; running them at the same time corrupts
chunks. `run.mjs --worktree-build` (auto-enabled when it detects a dev server listening on
`:3118`) builds in a throwaway git worktree with `node_modules` symlinked in (no reinstall),
serves that worktree's `out/`, and removes the worktree on exit:

```bash
WT=$(mktemp -d)/wt
git worktree add --detach "$WT" HEAD
ln -s "$PWD/node_modules" "$WT/frontend/node_modules"
( cd "$WT/frontend" && ./node_modules/.bin/next build )
# ...serve $WT/frontend/out and screenshot as usual, then: git worktree remove --force "$WT"
```

## Files
- `routes.json` — the manifest (see table above).
- `lib/manifest.mjs` — loads/validates `routes.json`, expands entries into per-theme shots.
- `tauri-mock.js` — browser stub of the Tauri runtime + sample data. **GENERATED** from
  `src-tauri/fixtures/demo/` (specs/0059's dev-fixtures dataset) by `build-mock.mjs` — don't
  hand-edit it. `src/__tests__/mock-contract.test.ts` fails the build if the mock ever
  answers a Tauri command that doesn't exist in `src-tauri/src/registry.rs`.
- `build-mock.mjs` — generates `tauri-mock.js`; also documents the URL params it reads.
- `serve-out.mjs` — tiny static server for the `out/` export (maps clean URLs → `*.html`).
- `lib/cdp.mjs` / `screenshot.mjs` — headless Chrome DevTools Protocol driver (launch, one
  browser many captures, the `shotReady` poll, deadline racing).
- `run.mjs` — `pnpm shots`.
- `diff.mjs` — `pnpm shots:diff`.
- `lib/control.mjs` — NDJSON control-socket client (real captures).
- `real.mjs` — `pnpm shots:real`.

## Regenerating the mock
After changing the fixture dataset (`src-tauri/fixtures/demo/`) or `build-mock.mjs`'s
command shapes, regenerate before re-running `pnpm shots`:
```bash
pnpm shots:mock   # node scripts/shots/build-mock.mjs
pnpm shots
```

## Limits
- **`pnpm shots` / `pnpm shots:diff` are mock-data only** — no real backend, calendar, or
  live audio. Only `pnpm shots:real` (against the actual `--demo` app) exercises those, and
  is the only way to capture `record-live`.
- The sidebar renders collapsed by default; add `sidebar=open` (headless) or set the
  manifest's `sidebar` field (either driver) to expand it.
- After UI changes, `pnpm shots` rebuilds the export automatically (skip with `--no-build`
  only when you know the export is already current).
- After fixture-dataset changes, regenerate `tauri-mock.js` first (see above) — the headless
  set will otherwise silently keep showing the old data.
- macOS path to Chrome is hardcoded in `lib/cdp.mjs`; override with `CHROME_PATH=...`.
- `pnpm shots:real` needs a real macOS window and the permissions listed above; it does not
  run in CI.
