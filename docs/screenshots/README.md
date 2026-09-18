# Screenshot sets (specs/0060)

Nixon keeps two screenshot sets, both driven from the same manifest
(`frontend/scripts/shots/routes.json`) and both showing only the fictional demo dataset
(`./dev-nixon.sh --demo`, specs/0059) — never a real user's meetings, people, or recordings.

## `docs/screenshots/headless/`
Rendered in headless Chrome against a static `next export`, with `window.__TAURI_INTERNALS__`
stubbed out (`frontend/scripts/shots/tauri-mock.js`). No GUI, no WindowServer, no real audio —
safe to run on a machine with no display (and in CI). This is a **committed** set — every
manifest entry that isn't `real_only`, in both themes — so a diff against `HEAD` catches
visual regressions. Regenerate with `pnpm shots`, review with `pnpm shots:diff` (writes a
contact sheet to `docs/screenshots/diff/index.html`, gitignored — it's a review artifact,
not something to commit).

## `docs/screenshots/real/`
Real PNGs of the actual "Dev Nixon" window (`./dev-nixon.sh --demo`), captured with macOS
`screencapture` and driven by a debug-only loopback control listener
(`frontend/src-tauri/src/dev_fixtures/control.rs`). This is the only set that can show
`record-live` (an in-progress recording with the VU meters actually moving) — the headless
mock has no audio pipeline to drive that. It's also what the top-level `README.md`'s
`## Screenshots` section links to. Regenerate with `pnpm shots:real`.

## The three commands (run from `frontend/`)

| Command | What it does | Needs |
|---|---|---|
| `pnpm shots` | Headless capture of every manifest entry (both themes) into `docs/screenshots/headless/`. | Chrome; builds/serves a static export first. |
| `pnpm shots:diff` | Pixelmatches each headless PNG against `HEAD` (or `--base <ref>`) and writes `docs/screenshots/diff/index.html`. `--strict` exits non-zero on any change, for gating. | A `docs/screenshots/headless/` set already on disk (from `pnpm shots`). |
| `pnpm shots:real` | Drives the running Dev Nixon window over the control listener and captures it with `screencapture` into `docs/screenshots/real/`. | `./dev-nixon.sh --demo` already running; Screen Recording permission for the terminal that launched it; Microphone/Audio Capture permission for the `record-live` shot. |

`pnpm shots:mock` regenerates `frontend/scripts/shots/tauri-mock.js` from the fixture dataset
(`frontend/src-tauri/fixtures/demo/`) — run it (then `pnpm shots`) after changing that
dataset. See `frontend/scripts/shots/README.md` for the full flag reference, URL params, and
the control protocol.

## Permissions for `pnpm shots:real`
- **Screen Recording**, granted to whatever terminal app (Terminal.app, iTerm, etc.) launches
  the dev app — without it, `screencapture -l <window>` fails with "could not create image
  from window" even though the app itself is running fine. System Settings → Privacy &
  Security → Screen Recording.
- **Microphone / Audio Capture**, needed only for the `record-live` shot: it starts a real
  recording via the control listener's `start_recording` command and plays a short
  synthesized clip through `afplay` so the level meters move during the take. System
  Settings → Privacy & Security → Microphone.
- Neither permission is needed for `pnpm shots` or `pnpm shots:diff` — both are headless and
  never open a real window.

## Fictional data only
Every image in both sets is rendered from the specs/0059 demo dataset (5 seeded meetings, 7
people, synthesized audio) — never a real recording, a real calendar entry, or a real
person's name. `pnpm shots` gets this from `tauri-mock.js` (generated from the same
fixtures); `pnpm shots:real` gets it because `./dev-nixon.sh --demo` seeds the `.debug`
profile with that dataset before the control listener drives it. Never point either pipeline
at a non-demo profile, and never hand-edit a captured PNG to redact something — re-seed and
re-capture instead.

## Owner procedure (before merging a UI change, or before cutting a release)
1. `cd frontend && pnpm shots` — confirms every manifest entry still renders, in both
   themes.
2. `pnpm shots:diff` — expect `0 changed` on a clean tree; look at the contact sheet and
   investigate anything else before committing `docs/screenshots/headless/*.png`.
3. Launch `./dev-nixon.sh --demo` from a terminal that already holds Screen Recording (and,
   for `record-live`, Microphone) permission.
4. `pnpm shots:real` — read every PNG in `docs/screenshots/real/` and reject any that shows a
   stale state (wrong tab, a stray DEV badge, an empty list); re-run with `--only <name>`
   for just those after fixing the cause.
5. Commit the refreshed `docs/screenshots/real/*.png` alongside the change. `release.sh`
   prints a non-blocking reminder when that directory is older than the tag being cut, as a
   backstop — it does not block the release.
