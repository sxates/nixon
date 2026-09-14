# Self-screenshot harness (`scripts/shots`)

Render the Nixon UI in **headless Chrome** and capture PNGs of any route — so the UI can
be validated without launching the Tauri app (which needs a GUI / WindowServer). Useful for
agents and CI-style visual checks. **Mock data only** — this does not exercise the real
backend, your meetings, the calendar, or live audio.

## Why it's not trivial
1. The app is a Tauri app: its React providers call `invoke(...)`. With no Tauri runtime,
   those calls hang and **wedge the renderer** — no tool can screenshot a wedged page.
   `tauri-mock.js` stubs `window.__TAURI_INTERNALS__` so the app boots in a plain browser.
2. The app stays **blank until onboarding reports complete** — the mock returns a completed
   `get_onboarding_status` (with a `model_status.parakeet` field) to pass that gate.
3. `next dev`'s HMR socket + the app's polling timers make the simple
   `chrome --screenshot --virtual-time-budget` hang. We drive Chrome over the **DevTools
   Protocol** (`screenshot.mjs`) and control the wait ourselves.
4. The app is configured `output: export`, so `next build` emits a static `out/` dir we serve
   with `serve-out.mjs` (no server, no HMR — deterministic).

## Files
- `tauri-mock.js` — browser stub of the Tauri runtime + the sample data screens render. **Edit
  the FIXTURES here to change what shows up.**
- `serve-out.mjs` — tiny static server for the `out/` export (maps clean URLs → `*.html`).
- `screenshot.mjs` — CDP screenshot driver: injects the mock, navigates, waits, captures.

## Usage

```bash
cd frontend

# 1) Build the static export.
#    If a dev server (./dev-nixon.sh) is RUNNING, build in an isolated git worktree instead,
#    so you don't corrupt its shared `.next` (see "Isolated build" below).
./node_modules/.bin/next build          # -> frontend/out/

# 2) Serve the export (any free port).
node scripts/shots/serve-out.mjs ./out 3211 &

# 3) Screenshot routes.
node scripts/shots/screenshot.mjs http://localhost:3211/            /tmp/home.png
node scripts/shots/screenshot.mjs http://localhost:3211/meetings    /tmp/meetings.png 1280 860
node scripts/shots/screenshot.mjs http://localhost:3211/record      /tmp/record.png
# each writes the PNG + a <out>.log breadcrumb ("OK <bytes>" on success)
```

### Isolated build (when a dev server is running)
`next build` and `next dev` both use `.next`; running them together corrupts chunks. Build in
a throwaway worktree with symlinked deps (no reinstall):

```bash
WT=$(mktemp -d)/wt
git worktree add --detach "$WT" HEAD
ln -s "$PWD/node_modules" "$WT/frontend/node_modules"
( cd "$WT/frontend" && ./node_modules/.bin/next build )
node scripts/shots/serve-out.mjs "$WT/frontend/out" 3211 &
# ...screenshot as above, then: git worktree remove --force "$WT"
```

## Limits
- Mock data only (no real meetings / calendar / audio / summaries).
- The sidebar renders collapsed by default (fresh profile). To show it expanded, set the
  relevant `localStorage` key in `tauri-mock.js` before the app reads it.
- After UI changes you must **rebuild** the export to re-screenshot.
- macOS path to Chrome is hardcoded; override with `CHROME_PATH=...`.
