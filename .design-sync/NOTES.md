# design-sync notes — Vinyl UI

## What this syncs
- Vinyl is an **app repo, not a published design system**. The synced surface is the
  shadcn/ui primitives under `frontend/src/components/ui/`.
- **Local bundle is now the full 22 components** (updated 2026-07-06): `frontend/.ds-entry.tsx`
  exports all 22, `cfg.componentSrcMap` maps all 22, `.ds-tw.config.js` `content` covers
  `src/components/ui/*.tsx` + all `.design-sync/previews/**`, and a preview `.tsx` exists for
  each. The original Jun-27 **pilot pushed only 6** (Button, Input, Select, Dialog, Tabs,
  Tooltip); the source scaffolding here is already the full set, so no expansion work remains —
  a `/design-sync` run picks up all 22.

## ✅ Full re-sync completed 2026-07-06 (all 22, fresh project)

- **Project:** "Vinyl UI" `636e117f-f939-4df1-8d78-f6f8690074fc` (pinned in `config.json`). The old
  `6ac6f2d1-…` id was **404 / gone** and the account had zero projects — created fresh.
- **All 22 primitives** built, render-checked (22/22 clean via system Chrome), graded `good`, and
  uploaded. Render check: total 22, bad 0, thin 0, variantsIdentical 0.
- **Config schema migration (current skill):** the old `entry` config key is **rejected** by the
  current strict validator — it's passed on the CLI now: `--entry ./frontend/.ds-entry.tsx`.
  Removed `entry` from `config.json`. Everything else validated.
- **Build commands that worked (from repo root):**
  - Stage: `cp -r <skill-base>/{package-build,package-validate,package-capture,resync}.mjs <skill-base>/{lib,storybook} .ds-sync/` then `(cd .ds-sync && npm i esbuild ts-morph @types/react)`.
  - **CSS (the crux):** recompile the Tailwind sheet first — `cd frontend && npx tailwindcss -c ./.ds-tw.config.js -i ./.ds-sync-styles/input.css -o ./.ds-sync-styles/vinyl-ui.css` (`cfg.cssEntry` = `.ds-sync-styles/vinyl-ui.css`). **`input.css` imports ALL FIVE brand fonts** (Archivo, Archivo Narrow, IBM Plex Sans, IBM Plex Mono, Courier Prime — specs/0057) via a remote Google Fonts `@import`, then `@import '../src/app/globals.css'`, then defines `--font-archivo`/`--font-archivo-narrow`/`--font-plex-sans`/`--font-plex-mono`/`--font-courier-prime`. All five must be present — don't regress to a subset.
  - Build: `node .ds-sync/package-build.mjs --config .design-sync/config.json --node-modules ./frontend/node_modules --entry ./frontend/.ds-entry.tsx --out ./ds-bundle`.
  - Validate/capture render check uses **system Chrome, not a Playwright browser download**:
    `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright` in `.ds-sync`, then run validate/capture with
    `DS_CHROMIUM_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"`.
- **Config `overrides` added:** `Alert: {cardMode: "column"}` (its stories were `[GRID_OVERFLOW]` — wider than a grid cell). The 6 overlay overrides (Dialog/Tooltip/DropdownMenu/Popover/Sheet/Command `cardMode:single`) carried over from the pilot and still render correctly (Sheet's dark area is the scrim; its right-side panel renders fine).

## Known render warns (expected — not new)
- **`[FONT_REMOTE]` "Archivo", "Archivo Narrow", "IBM Plex Sans", "IBM Plex Mono", "Courier Prime"** — loaded via the remote Google Fonts `@import` in `input.css`; they serve at runtime. Not a `[FONT_MISSING]`. No action.
- **`tokens: 2 missing, below threshold`** — 53 referenced / 124 defined, 2 undefined custom props under the warn threshold. Non-blocking.
- **`.d.ts parse check skipped`** — `typescript` not installed in `.ds-sync/node_modules` (optional). Add `npm i typescript` there to enable the emitted-`.d.ts` lint.

## Re-sync risks / watch-list
- **CSS is Tailwind-compiled + safelisted** (`.ds-tw.config.js`). Classes outside the safelist won't be styled in agent-built designs — widen the safelist if the design agent needs more utilities. **Always recompile `vinyl-ui.css` before a re-build** when component/preview classes change.
- **`input.css` fonts are a remote `@import`** — fully offline render environments fall back. If a brand font changes, update the `@import` URL AND the `--font-*` var names.
- **The compiled `vinyl-ui.css` + `.ds-sync/` + `ds-bundle/` are gitignored** and regenerated per run — never hand-maintained. Only the durable set under `.design-sync/` (config.json, NOTES.md, conventions.md, previews/) is committed.
- **conventions.md validated 2026-07-06** — every Button variant it names (default/secondary/outline/ghost/destructive/link/blue/green/red/gray) exists in `button.tsx`. Note the legacy naming: `variant="blue"` = `bg-brand` (terracotta "Join & Record"), `red` = `bg-record`. If button variants are renamed, update conventions.md.

## ⚠️ Before the next push (historical — superseded by the 2026-07-06 re-sync above)
- **The build workspace is gone (expected — gitignored machine-state).** `.ds-sync/`
  (`package-build.mjs`, its `node_modules`, staged skill scripts) and
  `frontend/.ds-sync-styles/` (compiled `vinyl-ui.css` + `input.css`) are all under
  `.gitignore:88-94` and are **regenerated by the `/design-sync` skill on each run** — they are
  NOT hand-maintained. `package-build.mjs` is staged by the skill, not committed.
- **Requires the `/design-sync` skill + design auth.** The bare `DesignSync` tool is only the
  transport; the skill is what compiles the React primitives into `window.VinylUI`, recompiles
  the Tailwind CSS, and orchestrates create-project → build → push. To push: authorize with
  `/design-login` (or `/login` with the subscription account), then run `/design-sync`.
- **`config.json` `projectId` is STALE** (`6ac6f2d1-…` belongs to a prior claude.ai account —
  not writable). Create a **fresh "Vinyl UI"** design-system project on the current account and
  let the skill write the new id back; do not push to the old id.

## Build setup (package shape, synth-ish entry)
- There is no component `dist`. A barrel entry `frontend/.ds-entry.tsx` re-exports the
  synced primitives; `cfg.entry` points at it so `PKG_DIR` resolves to `frontend/`
  (its `package.json` name is `nixon` — it was `vinyl` before specs/0057). Bundle global: `window.VinylUI`.
- Build: `node .ds-sync/package-build.mjs --config .design-sync/config.json --node-modules frontend/node_modules --out ./ds-bundle`
- `@/` alias resolves via `cfg.tsconfig` = `frontend/tsconfig.json` (esbuild tsconfig-paths plugin, baseUrl defaults to `.`).

## CSS (the crux — Tailwind, not a shipped stylesheet)
- Components are styled by a **compiled Tailwind stylesheet**, regenerated, NOT shipped by the repo:
  `cd frontend && npx tailwindcss -c ./.ds-tw.config.js -i ./.ds-sync-styles/input.css -o ./.ds-sync-styles/vinyl-ui.css`
  → `cfg.cssEntry = .ds-sync-styles/vinyl-ui.css`.
- `input.css` = `globals.css` (tokens) + a remote `@import` for the five brand fonts
  (Archivo, Archivo Narrow, IBM Plex Sans, IBM Plex Mono, Courier Prime) + definitions for
  `--font-archivo`, `--font-archivo-narrow`, `--font-plex-sans`, `--font-plex-mono` and
  `--font-courier-prime` (the app sets those vars via Next's font loader at runtime).
- **`.ds-tw.config.js` has a `safelist`** of semantic token + common layout utilities so the
  design agent's own markup is styleable (Tailwind purge would otherwise drop them).
- **MUST recompile this CSS before every build** when component or preview classes change,
  or when expanding the component set (add them to the `content` glob first).

## Render check
- No Chromium installed; used **system Chrome**: `export DS_CHROMIUM_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"` before validate/capture.
- Playwright JS installed in `.ds-sync` without browsers (`PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm i playwright`).

## Overlay overrides
- `Dialog`: `cardMode single`, viewport `660x420` (≥640 so the footer renders as a row, not stacked).
- `Tooltip`: `cardMode single`, viewport `320x200`, preview forces `<Tooltip open>`.

## Known render warns (expected — not new)
- `[FONT_REMOTE] "Source Sans 3"` — loaded via remote font-host `@import`; serves at runtime.
- `.d.ts parse check skipped` — `typescript` not in `.ds-sync/node_modules` (optional; `npm i typescript` there to enable the emitted-.d.ts lint).

## Re-sync risks / watch-list
- **CSS is purged + safelisted.** Classes outside the safelist won't be styled in
  agent-built designs. Widen the safelist in `.ds-tw.config.js` if the agent needs more.
- **Barrel + componentSrcMap list only the 6 pilot components.** To expand to all 22:
  add exports to `frontend/.ds-entry.tsx`, entries to `cfg.componentSrcMap`, the files to
  the Tailwind `content` glob, then recompile CSS + rebuild.
- **`.d.ts` is src-derived** (no real typed dist) — props are weaker than a typed library build.
- Font is a remote `@import`; fully offline render environments may fall back.
