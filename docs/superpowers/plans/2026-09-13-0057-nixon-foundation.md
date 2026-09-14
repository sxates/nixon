# Nixon Foundation (rename + theme system) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the product from Vinyl to Nixon everywhere a user or future contributor can see it, and replace the "Warm Editorial" theme with the Nixon token set, fonts, and a system-following light/dark theme switch — the foundation the signature components (Plan 2) and per-screen redesign (Plan 3) build on.

**Architecture:** The rename is mechanical: `APP_NAME`, Tauri/Cargo/npm manifests, scripts, user-visible strings, internal keys, docs, and the app icon change; the bundle identifier `ai.vinyl.app` does **not** (decision 11). The theme lands as (1) a wholesale replacement of the CSS-variable token blocks in `globals.css` plus new Tailwind color/font entries, (2) a `ThemeProvider` that applies `.dark` on `<html>` from a persisted `nixon.theme` preference (`light | dark | system`, default `system`) and the OS `prefers-color-scheme`, (3) an Appearance selector in Settings → General, and (4) a sweep of every hardcoded Tailwind palette class onto semantic tokens, enforced by a new grep gate in CI so dark mode cannot regress.

**Tech Stack:** Tauri 2 (Rust), Next.js 14 / React 18 / TypeScript, Tailwind 3 (`darkMode: ['class']`), shadcn/Radix primitives, Vitest + Testing Library (jsdom), `next/font/google`.

**Spec:** `specs/0057-nixon-rebrand-and-tape-deck-redesign.md` — read "Decisions locked" first; it overrides §2–§8 where they differ. This plan covers spec §7 **Phase A** and **Phase B** only. Plans 2 (Phase C: transport rail, VU, counter, reels, channel strip, global queue) and 3 (Phase D: per-screen) are written after this plan lands, on the same branch `feat/0057-nixon-rebrand`.

## Global Constraints

- Bundle identifiers stay **`ai.vinyl.app`** (prod) and **`ai.vinyl.app.debug`** (dev). Never touch `identifier` in either Tauri config, `data_migration.rs`, `secrets.rs`, or any `ai.vinyl.app` string. (Spec decision 11.)
- Everything else visible says **Nixon**: product name "Nixon", dev build "Dev Nixon", window titles, notifications, tray tooltip, About, onboarding, docs, script names, release tag prefix `nixon-v`, DMG `Nixon_<ver>_aarch64.dmg`, app bundle `Nixon.app` / `Dev Nixon.app`.
- References to **meetily** stay only where they name the upstream project being imported from or credited: `DatabaseImport/*` UI copy, the `LegacyDatabaseImport` homebrew paths (`/opt/homebrew/var/meetily/…`), `About.tsx:51` "Forked from meetily (MIT)", `CHANGELOG.md` history, `CLAUDE.md` fork section, `vad.rs` upstream comments, the parakeet model download host, and `ci-checks.yml:4`. Every other `meetily` string is renamed.
- Light ("Faceplate") is the default theme; the app follows the OS dark setting; Settings → General offers Faceplate / Deck / System. (Decision 1.)
- Fonts: Archivo (UI), Archivo Narrow (meter scales/micro labels), IBM Plex Sans (reading), IBM Plex Mono (timecodes), Courier Prime (transcript body + reel labels). Newsreader and Source Sans 3 are removed. (Decision 4.)
- `--radius` is **0.25rem**. Do not mass-replace `rounded-full` in this plan (that is per-screen work in Plan 3); only replace it where a task explicitly says so.
- File-size ratchet: `scripts/check-file-size.sh` must pass. No production file over 800 lines; allowlisted files may only shrink. New code goes in **new files**. Never run bare `cargo fmt` (it reflows allowlisted files past the ratchet); use `cargo fmt -- <changed files>` only.
- Definition of Done per task: the commands named in the task pass. Definition of Done for the plan: `cd frontend/src-tauri && cargo check --features metal && cargo clippy --features metal --all-targets -- -D warnings && cargo test --features metal` ; `cd frontend && pnpm lint && pnpm test && npx tsc --noEmit` ; `scripts/check-file-size.sh` ; `scripts/check-off-token-colors.sh` ; app launches via `frontend/dev-nixon.sh` and the owner smoke path (record → live transcript → summary) works in both themes.
- Commit after every task with the trailer:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01S81GbiVj2rm2JNeZqSn8vX
  ```

---

## File map

| Area | Files |
|---|---|
| Rename — Rust/config | `frontend/src-tauri/src/app_paths.rs:21`, `tauri.conf.json:3,15`, `tauri.dev.conf.json:4,11`, `frontend/src-tauri/Cargo.toml:2,4`, `frontend/package.json:2,16,17`, `notifications/types.rs`, `notifications/commands.rs:355,411`, `tray.rs:25`, `lib.rs:307,328`, `calendar/google/{mod,commands,oauth}.rs`, `calendar/commands.rs:31`, `database/setup.rs:39`, `audio/recording_preferences.rs:85-114`, `audio/capture/core_audio.rs:151`, `audio/decoder.rs:302`, `summary/summary_engine/sidecar.rs:117-324`, `diarization/accel.rs`, `diarization/real_eval.rs` |
| Rename — scripts | `release.sh`, `frontend/src-tauri/scripts/cargo-dev-sign.sh`, `frontend/dev-vinyl.sh`→`dev-nixon.sh`, `frontend/upgrade-vinyl.sh`→`upgrade-nixon.sh`, `frontend/build-dev-app.sh`, `frontend/build-gpu.sh`, `frontend/dev-gpu.sh`, `frontend/clean_run.sh`, `scripts/reset-notification-icon-cache.sh`, `scripts/inject_transcript.py` |
| Rename — frontend | 41 user-visible strings (spec inventory §1A), 14 internal keys (`vinyl.*`, `vinyl-*`, `vinyl:*`, `MeetilyRecoveryDB`), 3 test assertions in `components/__tests__/CalendarSettings.test.tsx` |
| Rename — docs | `README.md`, `CLAUDE.md`, `SETUP.md`, `CONTRIBUTING.md`, `PRIVACY_POLICY.md`, `ROADMAP.md`, `docs/MANUAL_SMOKE.md`, `CHANGELOG.md` (Unreleased entry only) |
| Icon | `frontend/src-tauri/icons/*` regenerated from `frontend/src-tauri/icons/nixon-icon.svg` |
| Theme tokens | `frontend/src/app/globals.css`, `frontend/tailwind.config.js`, `frontend/src/app/layout.tsx:41-52,249` |
| Theme runtime | Create `frontend/src/contexts/ThemeContext.tsx`, `frontend/src/contexts/__tests__/ThemeContext.test.tsx`; modify `layout.tsx`, `components/BlockNoteEditor/Editor.tsx:61`, `components/AISummary/BlockNoteSummaryView.tsx:272`; remove `"theme": "Light"` from both Tauri window configs |
| Appearance setting | Create `frontend/src/components/AppearanceSettings.tsx`, `frontend/src/components/__tests__/AppearanceSettings.test.tsx`; modify `app/settings/page.tsx:110-124` |
| Off-token sweep | 56 files (spec inventory §2A), grouped in Tasks 10–13 |
| Gate | Create `scripts/check-off-token-colors.sh`; modify `.github/workflows/ci-checks.yml` (frontend job, after `pnpm test`) |
| Delete | `frontend/src/app/design-preview/` (+ its Sidebar link at `Sidebar/index.tsx:199`), `frontend/src/components/RecordingStatusBar.tsx` (unmounted; spec §7 Phase C) |

---

### Task 1: The off-token color gate (written first; it is the acceptance test for Tasks 10–13)

**Files:**
- Create: `scripts/check-off-token-colors.sh`

**Interfaces:**
- Produces: a script that exits 1 and lists `file:line:match` when any production file under `frontend/src` uses a raw Tailwind palette class or hex color; exits 0 otherwise. Tasks 10–13 drive its count to zero; Task 14 adds it to CI.

- [ ] **Step 1: Write the script**

```bash
#!/bin/bash
# specs/0057 Phase B — semantic-token gate.
#
# Fails if any production frontend source uses a raw Tailwind palette color class
# (bg-red-500, text-gray-700, border-white, ring-blue-400, …) or a hex literal in a
# class/style, instead of the semantic tokens (bg-background, text-muted-foreground,
# bg-success/10, text-chart-2, …). Raw palette classes do not flip with `.dark`, so
# every one of them is a light-only bug the moment the Deck theme is on.
#
# Allowed exceptions (add sparingly, with a reason):
#   - a line containing `token-gate: allow` (e.g. a deliberate pure-white overlay)
#
# Usage: scripts/check-off-token-colors.sh        # exit 1 on violation
set -euo pipefail
cd "$(dirname "$0")/.."

PALETTE='slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|white|black'
PREFIX='bg|text|border|ring|from|to|via|fill|stroke|shadow|divide|outline|placeholder|decoration|caret|accent'
CLASS_RE="(^|[^A-Za-z0-9_-])(${PREFIX})-(${PALETTE})(-[0-9]{2,3})?([/[:space:]\"'\`)]|$)"
HEX_RE="(${PREFIX})-\[#[0-9A-Fa-f]{3,8}\]|(color|background|border-color|fill|stroke)[[:space:]]*:[[:space:]]*['\"]?#[0-9A-Fa-f]{3,8}"

files() {
  git ls-files frontend/src |
    grep -E '\.(ts|tsx|css)$' |
    grep -vE '(^|/)(__tests__|fixtures)/|\.(test|spec)\.tsx?$'
}

violations=0
while IFS= read -r f; do
  hits=$( { grep -nE "$CLASS_RE" "$f" || true; grep -nE "$HEX_RE" "$f" || true; } | grep -v 'token-gate: allow' || true)
  if [ -n "$hits" ]; then
    while IFS= read -r line; do
      echo "$f:$line"
      violations=$((violations + 1))
    done <<< "$hits"
  fi
done < <(files)

if [ "$violations" -gt 0 ]; then
  echo
  echo "check-off-token-colors: $violations raw palette/hex color usage(s). Use semantic tokens" >&2
  echo "(see specs/0057 §Decisions + frontend/tailwind.config.js colors)." >&2
  exit 1
fi
echo "check-off-token-colors: ok"
```

- [ ] **Step 2: Make it executable and run it — expect it to FAIL with roughly 320–350 hits**

Run: `chmod +x scripts/check-off-token-colors.sh && scripts/check-off-token-colors.sh | tail -3`
Expected: last line `check-off-token-colors: 3xx raw palette/hex color usage(s)…` and exit code 1. Record the number in the commit message; Tasks 10–13 drive it to 0.

- [ ] **Step 3: Sanity-check false positives**

Run: `scripts/check-off-token-colors.sh | grep -vE '(bg|text|border|ring|from|to|via|fill|stroke|shadow|divide|outline)-' | head`
Expected: only hex-literal lines (`globals.css` scrollbar `#d1d5db`/`#9ca3af`, `design-preview/page.tsx` `'#fff'`). If a semantic class like `text-brand` or `bg-record` appears, the regex is wrong — fix `CLASS_RE` before continuing.

- [ ] **Step 4: Commit**

```bash
git add scripts/check-off-token-colors.sh
git commit -m "chore(0057): add off-token color gate script (currently failing: N hits)"
```

---

### Task 2: Rename — Rust sources and manifests

**Files:**
- Modify: `frontend/src-tauri/src/app_paths.rs:21`, `frontend/src-tauri/tauri.conf.json:3,15`, `frontend/src-tauri/tauri.dev.conf.json:4,11`, `frontend/src-tauri/Cargo.toml:2,4`, `frontend/package.json:2,16,17`, and the Rust files listed in Step 3.

**Interfaces:**
- Produces: crate/binary name `nixon` (so `target/debug/nixon`), npm package `nixon`, `APP_NAME = "Nixon"`, env vars `NIXON_GOOGLE_CLIENT_ID`, `NIXON_GOOGLE_CLIENT_SECRET`, `NIXON_LLAMA_HELPER`, `NIXON_DIARIZATION_PROVIDER`, `NIXON_DIARIZATION_THREADS`, `NIXON_EVAL_*`, `NIXON_DEV_SIGNING_IDENTITY`, `NIXON_DEV_BUNDLE`; default recordings folder `~/Movies/nixon-recordings`; Core Audio tap name `nixon-audio-tap`. Task 3 (scripts) depends on the binary name and env var names.

- [ ] **Step 1: Write the failing test for `APP_NAME`**

Append to the existing `#[cfg(test)]` module in `frontend/src-tauri/src/app_paths.rs` (create one at the end of the file if there is none):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_nixon() {
        assert_eq!(APP_NAME, "Nixon");
    }

    #[test]
    fn fallback_app_data_dir_ends_with_app_name() {
        // Uninitialized (no Tauri app in tests): the fallback path is <data_dir>/<APP_NAME>.
        assert!(app_data_dir().ends_with("Nixon"));
    }
}
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal app_paths::tests -- --nocapture 2>&1 | tail -5`
Expected: `app_name_is_nixon` FAILS with `left: "Vinyl", right: "Nixon"`.

- [ ] **Step 3: Apply the rename with a guarded perl substitution (keeps `ai.vinyl.app`)**

From the repo root:

```bash
# Rust + manifests. The lookbehind/lookahead keep `ai.vinyl.app` and `ai.vinyl.app.debug`.
perl -pi -e 's/(?<!ai\.)\bvinyl\b(?!\.app)/nixon/g; s/\bVinyl\b/Nixon/g; s/\bVINYL_/NIXON_/g' \
  frontend/src-tauri/src/app_paths.rs \
  frontend/src-tauri/src/lib.rs \
  frontend/src-tauri/src/tray.rs \
  frontend/src-tauri/src/notifications/types.rs \
  frontend/src-tauri/src/notifications/commands.rs \
  frontend/src-tauri/src/notifications/system.rs \
  frontend/src-tauri/src/calendar/commands.rs \
  frontend/src-tauri/src/calendar/mod.rs \
  frontend/src-tauri/src/calendar/eventkit.rs \
  frontend/src-tauri/src/calendar/google/mod.rs \
  frontend/src-tauri/src/calendar/google/commands.rs \
  frontend/src-tauri/src/calendar/google/oauth.rs \
  frontend/src-tauri/src/database/setup.rs \
  frontend/src-tauri/src/database/repositories/voiceprints.rs \
  frontend/src-tauri/src/database/repositories/people.rs \
  frontend/src-tauri/src/audio/recording_saver.rs \
  frontend/src-tauri/src/audio/import.rs \
  frontend/src-tauri/src/audio/recording_recovery.rs \
  frontend/src-tauri/src/meetings/discard.rs \
  frontend/src-tauri/src/meetings/commands.rs \
  frontend/src-tauri/src/ollama/served_context.rs \
  frontend/src-tauri/src/diarization/accel.rs \
  frontend/src-tauri/src/diarization/real_eval.rs \
  frontend/src-tauri/src/diarization/sherpa.rs \
  frontend/src-tauri/src/summary/llm_wire.rs \
  frontend/src-tauri/src/summary/context_budget.rs \
  frontend/src-tauri/src/summary/llm_gate.rs \
  frontend/src-tauri/src/zoom/monitor.rs \
  frontend/src-tauri/src/zoom/mute.rs \
  frontend/src-tauri/src/audio/channel_writer.rs \
  frontend/src-tauri/tauri.conf.json \
  frontend/src-tauri/tauri.dev.conf.json \
  frontend/src-tauri/Cargo.toml \
  frontend/package.json

# meetily-derived runtime names (not upstream references):
perl -pi -e 's/meetily-recordings/nixon-recordings/g' frontend/src-tauri/src/audio/recording_preferences.rs frontend/src-tauri/src/audio/channel_writer.rs frontend/src-tauri/src/meetings/commands.rs
perl -pi -e 's/meetily-audio-tap/nixon-audio-tap/g' frontend/src-tauri/src/audio/capture/core_audio.rs
perl -pi -e 's/\.meetily_decode_/.nixon_decode_/g' frontend/src-tauri/src/audio/decoder.rs
perl -pi -e 's/MEETILY_LLAMA_HELPER/NIXON_LLAMA_HELPER/g' frontend/src-tauri/src/summary/summary_engine/sidecar.rs
perl -pi -e 's#config_dir\(\)/meetily/zoom\.json#config_dir()/<app-data>/zoom.json#' frontend/src-tauri/src/zoom/settings.rs
```

- [ ] **Step 4: Fix the two places the guard cannot see**

`frontend/src-tauri/src/app_paths.rs:4` doc comment: change ``hardcoded `"Meetily"`/`"meetily"` literal`` — leave as is (it documents upstream history). `frontend/src-tauri/Cargo.toml:7` `repository = "https://github.com/sxates/vinyl-app"` — leave as is until the GitHub repo is renamed (out of scope; note it in the CHANGELOG entry in Task 5). `frontend/src-tauri/src/notifications/system.rs:38` mentions "stale (Meetily) artwork" — leave (upstream reference).

- [ ] **Step 5: Verify no stray references remain**

Run: `grep -rnE 'Vinyl|(?<!ai\.)vinyl' frontend/src-tauri/src frontend/src-tauri/*.json frontend/src-tauri/Cargo.toml frontend/package.json -P | grep -v 'ai\.vinyl\.app' | grep -v 'sxates/vinyl-app'`
Expected: no output.

- [ ] **Step 6: Build, clippy, test**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal 2>&1 | tail -2 && cargo clippy --features metal --all-targets -- -D warnings 2>&1 | tail -2 && cargo test --features metal app_paths 2>&1 | tail -3`
Expected: `Finished` for check and clippy; `test result: ok` including `app_name_is_nixon`. Note: `Cargo.lock` updates itself on `cargo check` (package `vinyl` → `nixon`); include it in the commit.

- [ ] **Step 7: Update the pnpm lockfile name entry and verify install**

Run: `cd frontend && pnpm install --frozen-lockfile 2>&1 | tail -2`
Expected: succeeds (the root package name is not in the lockfile's importers key; if pnpm complains about `ERR_PNPM_OUTDATED_LOCKFILE`, run `pnpm install` without `--frozen-lockfile` once and commit `pnpm-lock.yaml`).

- [ ] **Step 8: Commit**

```bash
git add -A frontend/src-tauri/src frontend/src-tauri/tauri.conf.json frontend/src-tauri/tauri.dev.conf.json frontend/src-tauri/Cargo.toml frontend/package.json Cargo.lock frontend/pnpm-lock.yaml
git commit -m "feat(0057): rename product to Nixon in Rust sources and manifests (bundle id unchanged)"
```

---

### Task 3: Rename — shell scripts and release tooling

**Files:**
- Rename: `frontend/dev-vinyl.sh` → `frontend/dev-nixon.sh`, `frontend/upgrade-vinyl.sh` → `frontend/upgrade-nixon.sh`
- Modify: `release.sh`, `frontend/src-tauri/scripts/cargo-dev-sign.sh`, `frontend/build-dev-app.sh`, `frontend/build-gpu.sh`, `frontend/dev-gpu.sh`, `frontend/clean_run.sh`, `scripts/reset-notification-icon-cache.sh`, `scripts/inject_transcript.py`, `frontend/scripts/tauri-auto.js`

**Interfaces:**
- Consumes: binary `target/debug/nixon`, env vars `NIXON_*` from Task 2.
- Produces: `dev-nixon.sh`, `upgrade-nixon.sh`, release tag `nixon-vX.Y.Z`, DMG `Nixon_X.Y.Z_aarch64.dmg`, app `Nixon.app` / `Dev Nixon.app`.

- [ ] **Step 1: Rename the two owner-facing scripts with git**

```bash
git mv frontend/dev-vinyl.sh frontend/dev-nixon.sh
git mv frontend/upgrade-vinyl.sh frontend/upgrade-nixon.sh
```

- [ ] **Step 2: Apply the guarded rename to every script**

```bash
perl -pi -e 's/(?<!ai\.)\bvinyl\b(?!\.app)/nixon/g; s/\bVinyl\b/Nixon/g; s/\bVINYL_/NIXON_/g; s/dev-vinyl\.sh/dev-nixon.sh/g; s/upgrade-vinyl\.sh/upgrade-nixon.sh/g' \
  release.sh \
  frontend/src-tauri/scripts/cargo-dev-sign.sh \
  frontend/build-dev-app.sh \
  frontend/dev-nixon.sh \
  frontend/upgrade-nixon.sh \
  frontend/clean_run.sh \
  frontend/scripts/tauri-auto.js \
  scripts/reset-notification-icon-cache.sh
# upstream build scripts: only the banner strings (keeps the diff minimal for /sync-upstream)
perl -pi -e 's/\bMeetily\b/Nixon/g' frontend/build-gpu.sh frontend/dev-gpu.sh
# the legacy CSV injector targets OUR db now
perl -pi -e 's/\bMeetily\b/Nixon/g; s#"Application Support" / "Nixon"#"Application Support" / "ai.vinyl.app"#' scripts/inject_transcript.py
perl -pi -e 's/meetily-recordings/nixon-recordings/g' frontend/upgrade-nixon.sh
```

- [ ] **Step 3: Check the three lines the substitution must have produced in `release.sh`**

Run: `grep -nE 'TAG=|DMG=|APP=|name = "nixon"' release.sh`
Expected:
```
TAG="nixon-v${NEW}"
DMG="target/release/bundle/dmg/Nixon_${NEW}_aarch64.dmg"
... perl … $p=1 if /^name = "nixon"$/ …
APP="target/release/bundle/macos/Nixon.app"
```
And `grep -n 'BIN=' frontend/src-tauri/scripts/cargo-dev-sign.sh` → `BIN="$TARGET_DIR/debug/nixon"`.

- [ ] **Step 4: Dry-run the release script's parsing path**

Run: `./release.sh patch --dry-run 2>&1 | head -30` (the script supports `--dry-run`; if it prompts, answer `n`)
Expected: it prints the would-be tag `nixon-v1.20.1` and DMG path with `Nixon_`, and — if `.env.signing` still exports `VINYL_GOOGLE_CLIENT_ID` — dies with `Google client id missing … NIXON_GOOGLE_CLIENT_ID empty`. That die is correct: the owner must rename the variables in their gitignored `.env.signing` (`VINYL_GOOGLE_CLIENT_ID`→`NIXON_GOOGLE_CLIENT_ID`, `VINYL_GOOGLE_CLIENT_SECRET`→`NIXON_GOOGLE_CLIENT_SECRET`). Record that in the handover.

- [ ] **Step 5: Launch the dev app once through the renamed launcher**

Run: `cd frontend && ./dev-nixon.sh` (leave it up ~60s, confirm the window title is "Dev Nixon", then Ctrl-C).
Expected: window titled **Dev Nixon**; `cargo-dev-sign.sh` prints `[nixon] dev binary re-signed…` (or the ad-hoc no-op notice on a machine without the cert).

- [ ] **Step 6: Commit**

```bash
git add -A release.sh frontend/*.sh frontend/src-tauri/scripts frontend/scripts/tauri-auto.js scripts/
git commit -m "feat(0057): rename dev/upgrade/release scripts and tooling to Nixon"
```

---

### Task 4: Rename — frontend strings, internal keys, tests

**Files:**
- Modify: every `frontend/src` file in spec inventory §1A/§1B/§1C (41 visible strings, 14 keys, ~35 comments), `frontend/src/components/__tests__/CalendarSettings.test.tsx:41,110,128`, `frontend/src/services/indexedDBService.ts:34`
- Delete: `frontend/src/app/design-preview/` and its link in `frontend/src/components/Sidebar/index.tsx:199`

**Interfaces:**
- Produces: localStorage/event keys `nixon.meetings.viewMode`, `nixon.sidebar.collapsed`, `nixon-calendar-connect-dismissed`, `nixon-calendar-alerts-fired`, `nixon-join-and-record-meeting`, `nixon-day-agenda-cache`, `nixon-speaker-suggestion-dismissed`, `nixon:open-command-palette`, `nixon:resume-armed`, `nixon-record-prompt`; IndexedDB `NixonRecoveryDB`. (Renaming these resets only trivial per-machine UI preferences; the sidebar/view-mode defaults simply re-apply once.)

- [ ] **Step 1: Update the three test assertions first — expect FAIL**

In `frontend/src/components/__tests__/CalendarSettings.test.tsx` replace `Vinyl` with `Nixon` on lines 41, 110, 128 (the strings are quoted verbatim there).

Run: `cd frontend && pnpm test -- CalendarSettings 2>&1 | tail -6`
Expected: 3 failures (`Unable to find an element with the text: … Nixon …`).

- [ ] **Step 2: Delete the design-preview prototype page and its sidebar link**

```bash
git rm -r frontend/src/app/design-preview
```
In `frontend/src/components/Sidebar/index.tsx` remove the whole button/menu item whose `onClick={() => router.push('/design-preview')}` (around line 199 — delete the enclosing JSX element, not just the line). Then in `frontend/src/components/PermissionsModal.tsx:26` and `frontend/src/components/NoteEditor/NoteEditor.tsx:10` change the comment `design source: `/design-preview`` / `(design-preview)` to `design source: specs/0057 mockup`.

Run: `grep -rn "design-preview" frontend/src` → expected: no output.

- [ ] **Step 3: Apply the guarded rename to `frontend/src` (excluding the meetily-import UI)**

```bash
cd frontend
git ls-files src | grep -E '\.(ts|tsx)$' \
  | grep -vE 'components/DatabaseImport/|contexts/OnboardingContext\.tsx' \
  | xargs perl -pi -e 's/(?<!ai\.)\bvinyl\b(?!\.app)/nixon/g; s/\bVinyl\b/Nixon/g; s/\bVINYL_/NIXON_/g; s/MeetilyRecoveryDB/NixonRecoveryDB/g'
# DatabaseImport keeps "Meetily" as the upstream product name but its dialog title is ours:
perl -pi -e 's/Welcome to Vinyl!/Welcome to Nixon!/' src/components/DatabaseImport/LegacyDatabaseImport.tsx
cd ..
```

- [ ] **Step 4: Verify**

Run: `grep -rnP '(?<!ai\.)\bvinyl\b(?!\.app)|\bVinyl\b' frontend/src`
Expected: no output. Then `grep -rn -i meetily frontend/src | grep -v DatabaseImport | grep -v OnboardingContext` → expected: no output.

- [ ] **Step 5: Lint, typecheck, test**

Run: `cd frontend && pnpm lint 2>&1 | tail -3 && npx tsc --noEmit 2>&1 | tail -3 && pnpm test 2>&1 | tail -6`
Expected: lint clean, tsc clean, all tests pass including the three CalendarSettings assertions.

- [ ] **Step 6: Commit**

```bash
git add -A frontend/src
git commit -m "feat(0057): rename Nixon across frontend strings, storage keys and events; drop design-preview page"
```

---

### Task 5: Rename — docs, CLAUDE.md, CHANGELOG

**Files:**
- Modify: `README.md`, `CLAUDE.md`, `SETUP.md`, `CONTRIBUTING.md`, `PRIVACY_POLICY.md`, `ROADMAP.md`, `docs/MANUAL_SMOKE.md`, `docs/BUILDING.md`, `CHANGELOG.md` (Unreleased section only), `.claude/agents/*.md`, `.claude/commands/*.md` if present

- [ ] **Step 1: Apply the guarded rename to the living docs (not history)**

```bash
perl -pi -e 's/(?<!ai\.)\bvinyl\b(?!\.app)/nixon/g; s/\bVinyl\b/Nixon/g; s/\bVINYL_/NIXON_/g; s/dev-vinyl\.sh/dev-nixon.sh/g; s/upgrade-vinyl\.sh/upgrade-nixon.sh/g; s/meetily-recordings/nixon-recordings/g; s/\bvinyl-v/nixon-v/g' \
  README.md CLAUDE.md SETUP.md CONTRIBUTING.md PRIVACY_POLICY.md ROADMAP.md docs/MANUAL_SMOKE.md docs/BUILDING.md
ls .claude/agents/*.md .claude/commands/*.md 2>/dev/null | xargs -r perl -pi -e 's/(?<!ai\.)\bvinyl\b(?!\.app)/nixon/g; s/\bVinyl\b/Nixon/g; s/dev-vinyl\.sh/dev-nixon.sh/g; s/upgrade-vinyl\.sh/upgrade-nixon.sh/g'
```
Do **not** touch `specs/`, `docs/decisions/`, `docs/upstream/`, `docs/superpowers/`, or `CHANGELOG.md` history — they are dated records. `specs/0002` stays as the record of the Vinyl rebrand.

- [ ] **Step 2: Hand-edit the three places sed cannot get right**

1. `CLAUDE.md` top note: replace the `APP_NAME = "Vinyl"` paragraph with:
   > **`APP_NAME` = "Nixon"** — the product name is a *variable* (ADR-0002). Rebrands so far: meetily → Vinyl (`specs/0002`, 2026-06) → Nixon (`specs/0057`, 2026-09). The bundle id is still **`ai.vinyl.app`** (dev `ai.vinyl.app.debug`) — deliberately kept so no permission re-grant or data migration was needed; it is invisible to users and will move in a future identifier change (which must run `src/data_migration.rs` again). `APP_NAME` lives in `src/app_paths.rs`.
2. `CHANGELOG.md` `[Unreleased]` — replace `_(nothing yet)_` with:
   ```markdown
   ### Changed
   - **Rebrand: Vinyl → Nixon** (specs/0057). Product name, window/tray/notification titles,
     onboarding and settings copy, script names (`dev-nixon.sh`, `upgrade-nixon.sh`), release
     tag prefix (`nixon-v`), DMG/app bundle names, default recordings folder
     (`~/Movies/nixon-recordings` for fresh installs; an existing preference is kept), and the
     `.env.signing` variables (`NIXON_GOOGLE_CLIENT_ID` / `NIXON_GOOGLE_CLIENT_SECRET`).
     The bundle identifier `ai.vinyl.app` is unchanged, so existing data and permissions carry
     over untouched. The GitHub repository name is unchanged for now.
   - New app icon: two-reel silhouette on charcoal with an amber REC lamp.
   - **Theme: "Faceplate" (light) / "Deck" (dark)** replaces Warm Editorial. Follows the
     macOS appearance by default; Settings → General → Appearance overrides it. Fonts are now
     Archivo / Archivo Narrow / IBM Plex Sans / IBM Plex Mono / Courier Prime.
   ### Removed
   - The `/design-preview` prototype page and the unmounted `RecordingStatusBar` component.
   ```
   Also update the changelog preamble line `Git release tags are prefixed **`vinyl-`**` to say tags **from v1.21.0 on** are prefixed `nixon-`, earlier ones `vinyl-`.
3. `SETUP.md` `.env.signing` section: list the two renamed Google variables explicitly.

- [ ] **Step 3: Verify**

Run: `grep -rnP '(?<!ai\.)\bvinyl\b(?!\.app)|\bVinyl\b' README.md CLAUDE.md SETUP.md CONTRIBUTING.md PRIVACY_POLICY.md ROADMAP.md docs/MANUAL_SMOKE.md docs/BUILDING.md | grep -vE 'specs/0002|meetily → Vinyl|Vinyl → Nixon|vinyl-v[0-9]|prefixed .vinyl'`
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add README.md CLAUDE.md SETUP.md CONTRIBUTING.md PRIVACY_POLICY.md ROADMAP.md docs/MANUAL_SMOKE.md docs/BUILDING.md CHANGELOG.md .claude
git commit -m "docs(0057): rename living docs to Nixon; changelog entry; keep dated records as-is"
```

---

### Task 6: App icon

**Files:**
- Create: `frontend/src-tauri/icons/nixon-icon.svg`
- Regenerate: `frontend/src-tauri/icons/{32x32,64x64,128x128,128x128@2x,icon}.png`, `icon.icns`, `icon.ico`

- [ ] **Step 1: Write the 1024×1024 source SVG (spec §5, mockup component sheet)**

`frontend/src-tauri/icons/nixon-icon.svg`:
```svg
<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 512 512">
  <!-- charcoal squircle chassis with a 1px cream top bevel -->
  <rect x="0" y="0" width="512" height="512" rx="112" fill="#191C1F"/>
  <rect x="2" y="2" width="508" height="508" rx="110" fill="none" stroke="rgba(233,226,214,.18)" stroke-width="4"/>
  <!-- tape path, dipping over the head block -->
  <path d="M138 366 Q256 352 374 366" stroke="#E9E2D6" stroke-width="16" fill="none" stroke-linecap="square"/>
  <rect x="226" y="338" width="60" height="26" fill="#0E1012"/>
  <!-- two hubs -->
  <g fill="none" stroke="#E9E2D6" stroke-width="20">
    <circle cx="138" cy="236" r="78"/><circle cx="374" cy="236" r="78"/>
  </g>
  <g fill="none" stroke="#E9E2D6" stroke-width="12">
    <circle cx="138" cy="236" r="30"/><circle cx="374" cy="236" r="30"/>
  </g>
  <!-- three spoke slots per hub, 120° apart -->
  <g stroke="#E9E2D6" stroke-width="18" stroke-linecap="square">
    <path d="M138 206 V170"/><path d="M112 251 L81 269"/><path d="M164 251 L195 269"/>
    <path d="M374 206 V170"/><path d="M348 251 L317 269"/><path d="M400 251 L431 269"/>
  </g>
  <!-- REC lamp -->
  <circle cx="112" cy="430" r="14" fill="#F0A72A"/>
</svg>
```

- [ ] **Step 2: Rasterize with macOS Quick Look (no extra tools) and regenerate the icon set**

```bash
cd frontend/src-tauri/icons
qlmanage -t -s 1024 -o . nixon-icon.svg >/dev/null 2>&1 && mv nixon-icon.svg.png nixon-icon-1024.png
sips -g pixelWidth nixon-icon-1024.png   # expect 1024
cd ../..
pnpm tauri icon src-tauri/icons/nixon-icon-1024.png -o src-tauri/icons
rm src-tauri/icons/nixon-icon-1024.png
ls src-tauri/icons
```
Expected: `pnpm tauri icon` rewrites `32x32.png 64x64.png 128x128.png 128x128@2x.png icon.icns icon.ico icon.png` (plus Android/iOS folders — delete `src-tauri/icons/android` and `src-tauri/icons/ios` if generated; the app is macOS-only). If `qlmanage` writes a transparent-edged PNG larger than 1024, run `sips -Z 1024 nixon-icon-1024.png` before `tauri icon`.

- [ ] **Step 3: Look at it**

Run: `open frontend/src-tauri/icons/128x128.png` and `open frontend/src-tauri/icons/32x32.png`.
Expected: two cream reels on charcoal, amber dot bottom-left, legible at 32px. If the 32px reads as mush, edit the SVG stroke widths up (20→24 on the hubs) and repeat Step 2.

- [ ] **Step 4: Launch the dev app and check the Dock icon**

Run: `cd frontend && ./dev-nixon.sh` (30s, then Ctrl-C).
Expected: Dock shows the new icon. (macOS may cache the old one for the *installed* production app until `scripts/reset-notification-icon-cache.sh` runs — that is expected and out of scope here.)

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/icons
git commit -m "feat(0057): new Nixon app icon (two-reel silhouette) and regenerated icon set"
```

---

### Task 7: Tokens, fonts, Tailwind — the Faceplate/Deck palette

**Files:**
- Modify: `frontend/src/app/globals.css` (both `@layer base` token blocks, the heading/notes font rules, `.u-*` utilities, scrollbar colors, the three entrance keyframes), `frontend/tailwind.config.js` (fontFamily + colors), `frontend/src/app/layout.tsx:1-52,249`
- Modify: `frontend/src/components/TranscriptSettings.tsx:213`, `frontend/src/components/ModelSettingsModal.tsx:1082` (drop `animate-vibrate`)

**Interfaces:**
- Produces: CSS variables (light on `:root`, dark on `.dark`): `--background --foreground --card --card-foreground --popover --popover-foreground --primary --primary-foreground --secondary --secondary-foreground --muted --muted-foreground --accent --accent-foreground --destructive --destructive-foreground --border --input --ring --radius --brand --brand-foreground --record --record-foreground --success --success-foreground --chart-1..5 --panel --engrave --paper --paper-ink --key --well --walnut --meter-over --bevel-hi --bevel-lo`. Tailwind color names: `success`, `panel`, `engrave`, `paper`, `paper-ink`, `key`, `well`, `walnut`, `meter-over` (+ existing). Font utilities: `font-sans` (Archivo), `font-narrow`, `font-reading`, `font-mono` (Plex Mono), `font-type` (Courier Prime). CSS variables `--font-archivo --font-archivo-narrow --font-plex-sans --font-plex-mono --font-courier-prime`.

- [ ] **Step 1: Replace the two token blocks in `globals.css`**

Replace everything from `@layer base {` / `/* Warm Editorial — paper tones…` through the closing `}` of `.dark { … }` (currently lines ~78–151) with:

```css
@layer base {
  /* ---- FACEPLATE (light, default) — brushed putty aluminum, warmed toward the walnut
     cheek. Values are the specs/0057 round-2 mockup palette converted to HSL. ---- */
  :root {
    --background: 39 22% 87%;          /* #E6E1D8 */
    --foreground: 30 10% 11%;          /* #201D1A */
    --card: 40 33% 93%;                /* #F3EFE7 */
    --card-foreground: 30 10% 11%;
    --popover: 40 39% 95%;             /* #F8F5EF */
    --popover-foreground: 30 10% 11%;
    --primary: 30 10% 11%;             /* ink key, cream legend */
    --primary-foreground: 40 39% 95%;
    --secondary: 37 20% 84%;           /* = panel */
    --secondary-foreground: 30 10% 14%;
    --muted: 37 19% 82%;               /* #D9D2C7 */
    --muted-foreground: 30 7% 37%;     /* #665F58 — 6.1:1 on background */
    --accent: 36 17% 77%;              /* #CFC7BB */
    --accent-foreground: 30 10% 11%;
    --destructive: 4 59% 41%;          /* #A8332B */
    --destructive-foreground: 40 39% 95%;
    --border: 36 14% 73%;              /* #C4BCB0 */
    --input: 36 17% 77%;
    --ring: 31 84% 37%;
    --radius: 0.25rem;

    /* lamps — the only saturated colors on the panel */
    --brand: 31 84% 37%;               /* #B0620F amber, armed/active/queue */
    --brand-foreground: 40 39% 97%;
    --record: 3 63% 44%;               /* #B8302A REC */
    --record-foreground: 40 39% 97%;
    --success: 131 39% 30%;            /* #2F6B3A READY */
    --success-foreground: 40 39% 97%;

    /* channels: CH1 is always the owner mic (specs/0046) */
    --chart-1: 31 84% 37%;
    --chart-2: 186 52% 29%;            /* #246A72 */
    --chart-3: 28 10% 26%;             /* #4A433D */
    --chart-4: 22 62% 36%;             /* #954C23 */
    --chart-5: 266 30% 45%;            /* #6E5095 */

    /* deck materials */
    --panel: 37 20% 84%;               /* #DED8CE brushed rail */
    --engrave: 30 7% 34%;              /* #5C5650 silkscreen ink */
    --paper: 41 50% 94%;               /* #F7F2E7 typewriter paper (transcript, reel label) */
    --paper-ink: 27 12% 15%;           /* #2A2521 */
    --key: 39 27% 90%;                 /* #ECE7DE transport key face */
    --well: 40 39% 95%;                /* #F8F5EF recessed counter/meter well */
    --walnut: 22 38% 25%;              /* #5A3A28 */
    --meter-over: 3 63% 44%;
    --bevel-hi: 0 0% 100% / 0.85;
    --bevel-lo: 0 0% 0% / 0.14;
  }

  /* ---- DECK (dark) — anodized charcoal, cream silkscreen. Applied by ThemeProvider. ---- */
  .dark {
    --background: 30 8% 9%;            /* #1A1816 */
    --foreground: 39 32% 88%;          /* #EAE3D6 */
    --card: 15 6% 13%;                 /* #242120 */
    --card-foreground: 39 32% 88%;
    --popover: 24 6% 16%;              /* #2B2826 */
    --popover-foreground: 39 32% 90%;
    --primary: 39 32% 88%;             /* cream key, dark legend */
    --primary-foreground: 30 8% 9%;
    --secondary: 20 7% 16%;
    --secondary-foreground: 39 20% 80%;
    --muted: 20 8% 15%;                /* #2A2624 */
    --muted-foreground: 33 9% 57%;     /* #9C9388 — 5.2:1 on background */
    --accent: 17 7% 21%;               /* #383331 */
    --accent-foreground: 39 32% 90%;
    --destructive: 5 68% 54%;          /* #D9463A */
    --destructive-foreground: 40 30% 96%;
    --border: 17 6% 22%;               /* #3B3634 */
    --input: 12 5% 20%;
    --ring: 38 87% 55%;
    --radius: 0.25rem;

    --brand: 38 87% 55%;               /* #F0A72A */
    --brand-foreground: 30 10% 8%;
    --record: 5 74% 56%;               /* #E24A3D */
    --record-foreground: 40 30% 96%;
    --success: 127 29% 50%;            /* #5AA362 */
    --success-foreground: 30 10% 8%;

    --chart-1: 38 87% 55%;
    --chart-2: 185 45% 52%;            /* #4FB3BC */
    --chart-3: 39 26% 78%;             /* #D6CCB9 */
    --chart-4: 22 61% 52%;             /* #D0703A */
    --chart-5: 263 28% 68%;            /* #A896C4 */

    --panel: 20 7% 16%;                /* #2C2826 */
    --engrave: 36 10% 61%;             /* #A69E92 */
    --paper: 26 10% 14%;               /* #26221F */
    --paper-ink: 38 30% 84%;           /* #E3DACB */
    --key: 22 11% 15%;                 /* #292421 */
    --well: 0 6% 7%;                   /* #121010 */
    --walnut: 22 38% 21%;              /* #4A3021 */
    --meter-over: 5 74% 56%;
    --bevel-hi: 0 0% 100% / 0.06;
    --bevel-lo: 0 0% 0% / 0.45;
  }
}
```

- [ ] **Step 2: Fonts and utilities in `globals.css`**

1. Replace the `h1, h2, .font-display { font-family: var(--font-newsreader)… }` rule with:
   ```css
   h1, h2, .font-display {
     font-family: var(--font-archivo), "Helvetica Neue", -apple-system, sans-serif;
     font-weight: 600;
     letter-spacing: -0.011em;
   }
   ```
2. `.note-editor { font-family: var(--font-source-sans-3), … }` → `font-family: var(--font-plex-sans), "Helvetica Neue", -apple-system, sans-serif;` and `.note-editor h1, h2, h3` font-family → `var(--font-archivo), "Helvetica Neue", sans-serif`.
3. Scrollbar: `background: #d1d5db;` → `background: hsl(var(--border));`, `background: #9ca3af;` → `background: hsl(var(--muted-foreground));`, `scrollbar-color: #d1d5db transparent;` → `scrollbar-color: hsl(var(--border)) transparent;`.
4. Delete the `@keyframes vibrate`, `.animate-vibrate`, `@keyframes fade-in-up`, `.animate-fade-in-up`, and `.delay-75/.delay-100/.delay-150` blocks. Keep `fade-in` but change its duration to `140ms ease-out` and remove the `translateY` (opacity only — spec §2 "Motion is mechanical").
5. Replace the `@layer components` utilities:
   ```css
   @layer components {
     /* Engraved silkscreen label — every place a 1970 panel would have printed caps. */
     .u-section-label {
       @apply text-[10px] font-semibold uppercase tracking-[0.14em] text-engrave;
       text-shadow: 0 -1px 0 hsl(0 0% 100% / 0.75);
     }
     .dark .u-section-label {
       text-shadow: 0 1px 0 hsl(0 0% 100% / 0.06);
     }
     /* Section heading inside a document column ("Key points", "Decisions"). */
     .u-doc-heading {
       @apply font-display text-[18px] font-semibold text-foreground;
     }
     /* Muted secondary/metadata line (dates, durations, attendee summaries). */
     .u-meta {
       @apply text-[13px] text-muted-foreground;
     }
     /* Typewriter-on-paper: the transcript body and the reel label (specs/0057 decision 4). */
     .u-typed {
       @apply font-type text-[13.5px] leading-[1.5] text-paper-ink;
     }
   }
   ```
   Also update the `.summary-doc .bn-editor, .summary-doc .ProseMirror` rule to add `font-family: var(--font-plex-sans), "Helvetica Neue", sans-serif;`.

- [ ] **Step 3: Remove the two `animate-vibrate` usages**

`frontend/src/components/TranscriptSettings.tsx:213` and `frontend/src/components/ModelSettingsModal.tsx:1082`: change `'animate-vibrate text-red-500'` to `'text-destructive'` (the shake is gone; the lock button still turns red for the same duration — the `isLockButtonVibrating` state and timer stay as they are).

- [ ] **Step 4: Tailwind config**

In `frontend/tailwind.config.js`:
```js
  		fontFamily: {
  			sans: ['var(--font-archivo)', '"Helvetica Neue"', '-apple-system', 'sans-serif'],
  			narrow: ['var(--font-archivo-narrow)', '"Arial Narrow"', 'sans-serif'],
  			reading: ['var(--font-plex-sans)', '"Helvetica Neue"', 'sans-serif'],
  			mono: ['var(--font-plex-mono)', 'Menlo', 'monospace'],
  			type: ['var(--font-courier-prime)', '"Courier New"', 'Courier', 'monospace'],
  		},
```
(remove the `serif` entry) and in `colors` delete `tertiary: '#64748b',` and add after `chart`:
```js
  			success: {
  				DEFAULT: 'hsl(var(--success))',
  				foreground: 'hsl(var(--success-foreground))'
  			},
  			panel: 'hsl(var(--panel))',
  			engrave: 'hsl(var(--engrave))',
  			paper: {
  				DEFAULT: 'hsl(var(--paper))',
  				ink: 'hsl(var(--paper-ink))'
  			},
  			key: 'hsl(var(--key))',
  			well: 'hsl(var(--well))',
  			walnut: 'hsl(var(--walnut))',
  			'meter-over': 'hsl(var(--meter-over))',
```

- [ ] **Step 5: Fonts in `layout.tsx`**

Replace the import and the two font constants (lines 4, 41–52):
```tsx
import { Archivo, Archivo_Narrow, IBM_Plex_Sans, IBM_Plex_Mono, Courier_Prime } from 'next/font/google'

// specs/0057 — deck typography. Archivo is the panel/UI face (tabular figures for
// counters), Archivo Narrow the meter scales, Plex Sans the reading face, Plex Mono
// timecodes, Courier Prime the typewriter-on-paper transcript body.
const archivo = Archivo({ subsets: ['latin'], weight: ['400', '500', '600', '700'], variable: '--font-archivo' })
const archivoNarrow = Archivo_Narrow({ subsets: ['latin'], weight: ['400'], variable: '--font-archivo-narrow' })
const plexSans = IBM_Plex_Sans({ subsets: ['latin'], weight: ['400', '500', '600'], variable: '--font-plex-sans' })
const plexMono = IBM_Plex_Mono({ subsets: ['latin'], weight: ['400', '500'], variable: '--font-plex-mono' })
const courierPrime = Courier_Prime({ subsets: ['latin'], weight: ['400', '700'], variable: '--font-courier-prime' })
```
and line 249:
```tsx
      <body className={`${archivo.variable} ${archivoNarrow.variable} ${plexSans.variable} ${plexMono.variable} ${courierPrime.variable} font-sans antialiased`}>
```

- [ ] **Step 6: Build and look**

Run: `cd frontend && npx tsc --noEmit && pnpm lint 2>&1 | tail -2 && pnpm test 2>&1 | tail -4 && ./dev-nixon.sh`
Expected: clean; the app comes up in the putty Faceplate palette with Archivo headings and 4px radii. (`next/font/google` downloads the faces at build time; the first `pnpm dev` needs network once.) Check Today, Settings, and a meeting page. Some surfaces will still show hardcoded light-palette classes — that is Tasks 10–13.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/app/globals.css frontend/tailwind.config.js frontend/src/app/layout.tsx frontend/src/components/TranscriptSettings.tsx frontend/src/components/ModelSettingsModal.tsx
git commit -m "feat(0057): Faceplate/Deck token set, deck fonts, engraved label utility; drop Warm Editorial"
```

---

### Task 8: ThemeProvider — system-following with a persisted override

**Files:**
- Create: `frontend/src/contexts/ThemeContext.tsx`, `frontend/src/contexts/__tests__/ThemeContext.test.tsx`
- Modify: `frontend/src/app/layout.tsx` (wrap the tree), `frontend/src-tauri/tauri.conf.json:20`, `frontend/src-tauri/tauri.dev.conf.json:16` (remove `"theme": "Light"`), `frontend/src/components/BlockNoteEditor/Editor.tsx:61`, `frontend/src/components/AISummary/BlockNoteSummaryView.tsx:272`

**Interfaces:**
- Produces:
  ```ts
  export type ThemePreference = 'light' | 'dark' | 'system';
  export type ResolvedTheme = 'light' | 'dark';
  export const THEME_STORAGE_KEY = 'nixon.theme';
  export function ThemeProvider({ children }: { children: React.ReactNode }): JSX.Element;
  export function useTheme(): { preference: ThemePreference; resolved: ResolvedTheme; setPreference: (p: ThemePreference) => void };
  ```
  Side effect: `document.documentElement.classList` contains `dark` iff `resolved === 'dark'`. Task 9 (AppearanceSettings) consumes `useTheme`.

- [ ] **Step 1: Write the failing tests**

`frontend/src/contexts/__tests__/ThemeContext.test.tsx`:
```tsx
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, render, screen, fireEvent } from '@testing-library/react';
import { ThemeProvider, useTheme, THEME_STORAGE_KEY } from '@/contexts/ThemeContext';

// specs/0057 decision 1 — light default, follow the OS, manual override persisted.

type MQ = { matches: boolean; listeners: Array<(e: { matches: boolean }) => void> };
let mq: MQ;

function installMatchMedia(prefersDark: boolean) {
  mq = { matches: prefersDark, listeners: [] };
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: query === '(prefers-color-scheme: dark)' ? mq.matches : false,
    media: query,
    addEventListener: (_: string, cb: (e: { matches: boolean }) => void) => mq.listeners.push(cb),
    removeEventListener: (_: string, cb: (e: { matches: boolean }) => void) => {
      mq.listeners = mq.listeners.filter((l) => l !== cb);
    },
  })) as unknown as typeof window.matchMedia;
}

function Probe() {
  const { preference, resolved, setPreference } = useTheme();
  return (
    <div>
      <span data-testid="pref">{preference}</span>
      <span data-testid="resolved">{resolved}</span>
      <button onClick={() => setPreference('dark')}>dark</button>
      <button onClick={() => setPreference('light')}>light</button>
      <button onClick={() => setPreference('system')}>system</button>
    </div>
  );
}

beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove('dark');
});
afterEach(() => vi.restoreAllMocks());

describe('ThemeProvider', () => {
  it('defaults to system and resolves light when the OS is light', () => {
    installMatchMedia(false);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('pref').textContent).toBe('system');
    expect(screen.getByTestId('resolved').textContent).toBe('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('resolves dark and sets the .dark class when the OS is dark', () => {
    installMatchMedia(true);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('follows OS changes while preference is system', () => {
    installMatchMedia(false);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    act(() => { mq.matches = true; mq.listeners.forEach((l) => l({ matches: true })); });
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('a manual override wins over the OS and persists', () => {
    installMatchMedia(true);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    fireEvent.click(screen.getByText('light'));
    expect(screen.getByTestId('resolved').textContent).toBe('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('light');
    // OS flips to dark — ignored while overridden
    act(() => { mq.listeners.forEach((l) => l({ matches: true })); });
    expect(screen.getByTestId('resolved').textContent).toBe('light');
  });

  it('reads a persisted preference on mount and tolerates garbage', () => {
    installMatchMedia(false);
    localStorage.setItem(THEME_STORAGE_KEY, 'dark');
    const { unmount } = render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('resolved').textContent).toBe('dark');
    unmount();
    localStorage.setItem(THEME_STORAGE_KEY, 'neon');
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByTestId('pref').textContent).toBe('system');
  });
});
```

- [ ] **Step 2: Run — expect FAIL (module not found)**

Run: `cd frontend && pnpm test -- ThemeContext 2>&1 | tail -5`
Expected: `Failed to resolve import "@/contexts/ThemeContext"`.

- [ ] **Step 3: Implement `ThemeContext.tsx`**

```tsx
'use client';

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';

/**
 * specs/0057 decision 1 — Faceplate (light) is the default look; the app follows the
 * macOS appearance unless the user overrides it in Settings → General → Appearance.
 *
 * The resolved theme is applied as the `dark` class on <html> (Tailwind
 * `darkMode: ['class']`, and the `.dark` token block in globals.css). Nothing else in
 * the tree should touch that class.
 *
 * Note: the Tauri window config must NOT pin `"theme": "Light"` — that forces the
 * WebView's prefers-color-scheme to light and `system` would never resolve dark.
 */
export type ThemePreference = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'nixon.theme';
const DARK_QUERY = '(prefers-color-scheme: dark)';

interface ThemeContextValue {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (p: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function readStoredPreference(): ThemePreference {
  try {
    const v = localStorage.getItem(THEME_STORAGE_KEY);
    return v === 'light' || v === 'dark' || v === 'system' ? v : 'system';
  } catch {
    return 'system';
  }
}

function osPrefersDark(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(DARK_QUERY).matches
    : false;
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => readStoredPreference());
  const [systemDark, setSystemDark] = useState<boolean>(() => osPrefersDark());

  // Track the OS appearance. Only matters while preference === 'system', but keeping the
  // listener always-on means switching back to System is instant and correct.
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia(DARK_QUERY);
    const onChange = (e: { matches: boolean }) => setSystemDark(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);

  const resolved: ResolvedTheme =
    preference === 'system' ? (systemDark ? 'dark' : 'light') : preference;

  useEffect(() => {
    document.documentElement.classList.toggle('dark', resolved === 'dark');
  }, [resolved]);

  const setPreference = useCallback((p: ThemePreference) => {
    setPreferenceState(p);
    try {
      localStorage.setItem(THEME_STORAGE_KEY, p);
    } catch {
      /* private mode / quota — the in-memory preference still applies this session */
    }
  }, []);

  const value = useMemo(() => ({ preference, resolved, setPreference }), [preference, resolved, setPreference]);
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme must be used within a ThemeProvider');
  return ctx;
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cd frontend && pnpm test -- ThemeContext 2>&1 | tail -5`
Expected: 5 passed.

- [ ] **Step 5: Mount it, un-pin the Tauri window theme, and make BlockNote follow it**

1. `layout.tsx`: `import { ThemeProvider } from '@/contexts/ThemeContext'` and wrap the outermost provider: `<ThemeProvider><RecordingStateProvider>…</RecordingStateProvider></ThemeProvider>` (ThemeProvider is outermost so onboarding is themed too). Add `suppressHydrationWarning` to `<html lang="en" suppressHydrationWarning>` (the `dark` class is applied client-side).
2. Delete the line `"theme": "Light",` from `frontend/src-tauri/tauri.conf.json` (windows[0]) and `frontend/src-tauri/tauri.dev.conf.json` (windows[0]).
3. `components/BlockNoteEditor/Editor.tsx`: add `import { useTheme } from '@/contexts/ThemeContext'`, inside the component `const { resolved } = useTheme();`, and change `theme="light"` → `theme={resolved}`. Same three edits in `components/AISummary/BlockNoteSummaryView.tsx` (line 272).
4. Any test that renders `Editor` or `BlockNoteSummaryView` without a provider will now throw — run `pnpm test` and, for each failure, wrap that test's render in `<ThemeProvider>` (import from `@/contexts/ThemeContext`).

- [ ] **Step 6: Verify end to end**

Run: `cd frontend && npx tsc --noEmit && pnpm test 2>&1 | tail -4 && ./dev-nixon.sh`
Expected: tests pass. With macOS System Settings → Appearance set to Dark, the app comes up in Deck; flip the OS to Light and the app follows live. The summary editor text is readable in both.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/contexts/ThemeContext.tsx frontend/src/contexts/__tests__/ThemeContext.test.tsx frontend/src/app/layout.tsx frontend/src-tauri/tauri.conf.json frontend/src-tauri/tauri.dev.conf.json frontend/src/components/BlockNoteEditor/Editor.tsx frontend/src/components/AISummary/BlockNoteSummaryView.tsx
git commit -m "feat(0057): ThemeProvider — follow macOS appearance, persisted override; BlockNote follows theme"
```

---

### Task 9: Appearance selector in Settings → General

**Files:**
- Create: `frontend/src/components/AppearanceSettings.tsx`, `frontend/src/components/__tests__/AppearanceSettings.test.tsx`
- Modify: `frontend/src/app/settings/page.tsx:110-124` (mount it first in the General tab)

**Interfaces:**
- Consumes: `useTheme()` from Task 8.
- Produces: `export function AppearanceSettings(): JSX.Element` — a radiogroup labelled "Appearance" with options `Faceplate`, `Deck`, `System` (values `light | dark | system`).

- [ ] **Step 1: Write the failing test**

`frontend/src/components/__tests__/AppearanceSettings.test.tsx`:
```tsx
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ThemeProvider, THEME_STORAGE_KEY } from '@/contexts/ThemeContext';
import { AppearanceSettings } from '@/components/AppearanceSettings';

beforeEach(() => {
  localStorage.clear();
  window.matchMedia = vi.fn().mockImplementation((q: string) => ({
    matches: false, media: q, addEventListener: vi.fn(), removeEventListener: vi.fn(),
  })) as unknown as typeof window.matchMedia;
});

describe('AppearanceSettings', () => {
  it('shows three options with System selected by default', () => {
    render(<ThemeProvider><AppearanceSettings /></ThemeProvider>);
    const group = screen.getByRole('radiogroup', { name: /appearance/i });
    expect(group).toBeTruthy();
    expect(screen.getByRole('radio', { name: /system/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /faceplate/i })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('radio', { name: /deck/i })).toHaveAttribute('aria-checked', 'false');
  });

  it('selecting Deck persists dark and applies the class', () => {
    render(<ThemeProvider><AppearanceSettings /></ThemeProvider>);
    fireEvent.click(screen.getByRole('radio', { name: /deck/i }));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(screen.getByRole('radio', { name: /deck/i })).toHaveAttribute('aria-checked', 'true');
  });
});
```
(`toHaveAttribute` comes from `@testing-library/jest-dom`; check `frontend/vitest.setup.ts` imports it — it does for the existing component tests. If not, add `import '@testing-library/jest-dom/vitest'` there.)

- [ ] **Step 2: Run — expect FAIL**

Run: `cd frontend && pnpm test -- AppearanceSettings 2>&1 | tail -4`
Expected: module not found.

- [ ] **Step 3: Implement**

`frontend/src/components/AppearanceSettings.tsx`:
```tsx
'use client';

import React from 'react';
import { useTheme, type ThemePreference } from '@/contexts/ThemeContext';
import { cn } from '@/lib/utils';

// specs/0057 decision 1 — Faceplate (light) / Deck (dark) / System. Rendered as a
// three-position selector, not a switch: a deck has a labelled position for each state.
const OPTIONS: Array<{ value: ThemePreference; label: string; hint: string }> = [
  { value: 'light', label: 'Faceplate', hint: 'Brushed aluminum, ink labels' },
  { value: 'dark', label: 'Deck', hint: 'Charcoal chassis, cream silkscreen' },
  { value: 'system', label: 'System', hint: 'Follow the macOS appearance' },
];

export function AppearanceSettings() {
  const { preference, setPreference } = useTheme();
  return (
    <section className="space-y-3">
      <div>
        <h2 id="appearance-label" className="u-section-label">Appearance</h2>
        <p className="u-meta mt-1">Which face the machine wears. System follows macOS.</p>
      </div>
      <div role="radiogroup" aria-labelledby="appearance-label" className="grid grid-cols-3 gap-2">
        {OPTIONS.map((opt) => {
          const selected = preference === opt.value;
          return (
            <button
              key={opt.value}
              type="button"
              role="radio"
              aria-checked={selected}
              onClick={() => setPreference(opt.value)}
              className={cn(
                'flex flex-col items-start gap-1 rounded-md border px-3 py-2 text-left transition-colors',
                selected
                  ? 'border-brand bg-card text-foreground'
                  : 'border-border bg-panel text-muted-foreground hover:bg-accent',
              )}
            >
              <span className="flex items-center gap-2 text-sm font-semibold">
                <span
                  aria-hidden
                  className={cn('inline-block h-2 w-2 rounded-full', selected ? 'bg-brand' : 'bg-border')}
                />
                {opt.label}
              </span>
              <span className="text-xs">{opt.hint}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cd frontend && pnpm test -- AppearanceSettings 2>&1 | tail -4` → 2 passed.

- [ ] **Step 5: Mount in Settings → General (first item)**

In `frontend/src/app/settings/page.tsx` add `import { AppearanceSettings } from '@/components/AppearanceSettings';` and insert `<AppearanceSettings />` as the first child of `<TabsContent value="general" className="space-y-8">` (before `<PreferenceSettings />`). Update the comment above it: `General order: Appearance → Notifications → Recording permissions → Calendar → Your email`.

- [ ] **Step 6: Verify**

Run: `cd frontend && npx tsc --noEmit && pnpm lint 2>&1 | tail -2 && ./dev-nixon.sh` → Settings → General shows the selector; choosing Deck flips the whole app immediately and survives a relaunch.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/components/AppearanceSettings.tsx frontend/src/components/__tests__/AppearanceSettings.test.tsx frontend/src/app/settings/page.tsx
git commit -m "feat(0057): Appearance selector (Faceplate / Deck / System) in Settings → General"
```

---

## The off-token sweep (Tasks 10–13)

Shared mapping — apply it literally; when a line does not fit, pick the nearest semantic *meaning*, not the nearest hue:

| Raw class family | Replace with |
|---|---|
| `text-gray/slate/zinc/neutral/stone-{400,500}` | `text-muted-foreground` |
| `text-gray/slate/…-{600,700,800,900}`, `text-black` | `text-foreground` |
| `bg-white` | `bg-card` (surfaces) or `bg-background` (page ground) |
| `bg-gray/slate-{50,100}` | `bg-muted` |
| `bg-gray/slate-{200,300}` | `bg-accent` |
| `border-gray/slate-{200,300}`, `divide-gray-*` | `border-border` / `divide-border` |
| `ring-*-{n}` | `ring-ring` |
| `text-white` on a colored button | `text-primary-foreground` / `text-record-foreground` / `text-brand-foreground` / `text-success-foreground` to match that button's `bg-*` token |
| `green/emerald-*` (status: done, ready, connected, saved) | `text-success`, `bg-success/10`, `border-success/30` |
| `red/rose-*` (errors, destructive actions) | `text-destructive`, `bg-destructive/10`, `border-destructive/30` |
| `red-*` on **recording** controls specifically | `record` tokens (`bg-record text-record-foreground`) |
| `amber/yellow/orange-*` (warnings, pending, stars, DEV badge) | `text-brand`, `bg-brand/10`, `border-brand/30` |
| `blue/indigo/sky-*` (informational, links, "active") | `brand` tokens (the deck has one accent) |
| `violet/purple/cyan/fuchsia/lime/teal-*` (speaker/channel colors only) | `chart-1..5` |
| `shadow-{sm,md,lg,xl}` on **panels/cards** | keep `shadow-sm` (spec decision 3 allows a real shadow); leave dialog/popover shadows as is |
| hex `'#fff'` inline styles | delete the style or use `hsl(var(--card))` |

Speaker colors (Task 10) cycle `chart-1..5` and repeat; CH1 (`chart-1`) is the owner.

The gate `scripts/check-off-token-colors.sh` is the acceptance test for each task: run it, and the only remaining hits must be in files belonging to a *later* task. Each task ends with `pnpm lint && pnpm test && npx tsc --noEmit` and a launch to eyeball the touched surfaces **in both themes** (Settings → Appearance).

### Task 10: Sweep — primitives, speaker colors, button variants, DevBadge, global CSS

**Files:**
- Modify: `frontend/src/lib/speaker-colors.ts`, `frontend/src/components/ui/button.tsx:22`, `frontend/src/components/ui/sheet.tsx`, `frontend/src/components/ui/dialog.tsx`, `frontend/src/components/DevBadge.tsx`, `frontend/src/components/People/PersonAvatar.tsx`, `frontend/src/components/AvatarStack.tsx`, `frontend/src/components/EditableTitle.tsx`, `frontend/src/components/EmptyStateSummary.tsx`, `frontend/src/components/TranscriptEmptyState.tsx`, `frontend/src/components/Sidebar/index.tsx`, `frontend/src/components/ConfidenceIndicator.tsx`, `frontend/src/components/ActionItems/ActionItemRow.tsx`, `frontend/src/app/people/page.tsx`
- Delete: `frontend/src/components/RecordingStatusBar.tsx` (unmounted — verify with `grep -rn RecordingStatusBar frontend/src` → only its own file)

- [ ] **Step 1: Write the failing test for speaker colors**

`frontend/src/lib/__tests__/speaker-colors.test.ts` (create; there is no existing test):
```ts
import { describe, it, expect } from 'vitest';
import { speakerColorClass, speakerBgClass } from '@/lib/speaker-colors';

// specs/0057 — speaker/channel colors come from the chart tokens so they flip with the
// theme; CH1 (the owner mic, specs/0046) is always chart-1. Signatures unchanged.
describe('speaker-colors', () => {
  it('uses only chart tokens', () => {
    const keys = ['You', 'a', 'b', 'c', 'd', 'e', 'f', 'g'];
    for (const k of keys) {
      expect(speakerColorClass(k)).toMatch(/^text-chart-[1-5]$/);
      expect(speakerBgClass(k)).toMatch(/^bg-chart-[1-5]$/);
    }
  });
  it('falls back to muted tokens for a null key', () => {
    expect(speakerColorClass(null)).toBe('text-muted-foreground');
    expect(speakerBgClass(null)).toBe('bg-muted');
  });
});
```
If the real exports are named differently, read `frontend/src/lib/speaker-colors.ts` and use its exported names — the assertion is about the class *set*, not the names.

- [ ] **Step 2: Run — expect FAIL** (`pnpm test -- speaker-colors`).

- [ ] **Step 3: Implement**

In `speaker-colors.ts` replace the two arrays with
```ts
const SPEAKER_COLOR_CLASSES = ['text-chart-1', 'text-chart-2', 'text-chart-3', 'text-chart-4', 'text-chart-5'] as const;
const SPEAKER_BG_CLASSES = ['bg-chart-1', 'bg-chart-2', 'bg-chart-3', 'bg-chart-4', 'bg-chart-5'] as const;
```
keep index 0 pinned to the local user exactly as the existing code does, and change the null fallbacks `'text-gray-700'` → `'text-muted-foreground'`, `'bg-gray-400'` → `'bg-muted'`. `tailwind.config.js` already scans `src/lib`, so the literals survive purge.

`button.tsx`: `green: "bg-green-700 text-white shadow-sm hover:bg-green-700/90"` → `green: "bg-success text-success-foreground shadow-sm hover:bg-success/90"`. (Variant *names* `blue/green/red/gray` stay this plan — renaming them touches ~30 call sites and belongs to Plan 3.)

`DevBadge.tsx`: both `bg-amber-100 text-amber-800 border border-amber-300` → `bg-brand/15 text-brand border border-brand/40`.

The remaining files in this task: apply the mapping table line by line. Then `git rm frontend/src/components/RecordingStatusBar.tsx`.

- [ ] **Step 4: Verify**

Run: `scripts/check-off-token-colors.sh | grep -E 'lib/speaker-colors|ui/|DevBadge|PersonAvatar|AvatarStack|EditableTitle|EmptyStateSummary|TranscriptEmptyState|Sidebar/|ConfidenceIndicator|ActionItemRow|people/page'` → no output. `cd frontend && pnpm lint && pnpm test && npx tsc --noEmit` clean. Launch, open People and a meeting transcript in both themes — speaker names carry channel colors.

- [ ] **Step 5: Commit**

```bash
git add -A frontend/src/lib frontend/src/components/ui frontend/src/components frontend/src/app/people
git commit -m "refactor(0057): speaker colors on chart tokens; sweep primitives/badges to semantic tokens; drop RecordingStatusBar"
```

### Task 11: Sweep — recording, transcript, meeting, Today surfaces

**Files:**
- Modify: `frontend/src/components/RecordingControls.tsx`, `AudioLevelMeter.tsx`, `GlobalRecordingBar.tsx` (the `tone="dark"` ParticipantsPopover prop → remove the prop and delete the `tone` branch in `Participants/ParticipantsPopover.tsx:25,43`), `app/_components/TranscriptPanel.tsx`, `VirtualizedTranscriptView.tsx`, `Participants/ParticipantsPanel.tsx`, `Today/TimelineBlock.tsx`, `Today/TodayHeader.tsx`, `MeetingDetails/SummaryPanel.tsx`, `MeetingDetails/RetranscribeDialog.tsx`, `AISummary/index.tsx`, `DeferredBacklog/BacklogDetailPopover.tsx`, `lib/recordingNotification.tsx`, `TranscriptRecovery/TranscriptRecovery.tsx`

- [ ] **Step 1: Apply the mapping table.** Specific calls: the amber backpressure banner in `TranscriptPanel.tsx:156` (`bg-amber-50 …`) → `bg-brand/10 border-brand/30 text-foreground`; `RecordingControls.tsx:494` red device-error alert → `destructive` tokens; `TimelineBlock.tsx` `bg-emerald-50 text-emerald-700` → `bg-success/10 text-success`; `AudioLevelMeter.tsx:34,66` bar colors → `bg-success` / `bg-brand` / `bg-record` by level zone (its replacement is Plan 2; keep it working).

- [ ] **Step 2: Verify.** `scripts/check-off-token-colors.sh | grep -E 'RecordingControls|AudioLevelMeter|GlobalRecordingBar|TranscriptPanel|VirtualizedTranscriptView|Participants/|Today/|MeetingDetails/|AISummary/|DeferredBacklog/|recordingNotification|TranscriptRecovery'` → no output. `pnpm lint && pnpm test && npx tsc --noEmit` clean. Launch: record 30s, watch the live transcript and level meter, stop, open the meeting — in both themes.

- [ ] **Step 3: Commit** — `git commit -am "refactor(0057): sweep recording/transcript/meeting/Today surfaces to semantic tokens"`

### Task 12: Sweep — settings and model managers

**Files:**
- Modify: `frontend/src/components/ChunkProgressDisplay.tsx`, `WhisperModelManager.tsx`, `ParakeetModelManager.tsx`, `BuiltInModelManager.tsx`, `ModelSettingsModal.tsx`, `app/_components/SettingsModal.tsx`, `LanguageSelection.tsx`, `DeviceSelection.tsx`, `BluetoothPlaybackWarning.tsx`, `BetaSettings.tsx`, `RecordingSettings.tsx`, `TranscriptSettings.tsx`, `CalendarSettings.tsx`, `AudioBackendSelector.tsx`, `TemplateSettings/TemplateSettings.tsx`, `ComplianceNotification.tsx`

- [ ] **Step 1: Apply the mapping table.** `ChunkProgressDisplay.tsx` (28 hits) is a status ladder — map by state: pending → `muted`, running → `brand`, done → `success`, failed → `destructive`. Model "downloaded/ready" badges → `success`; "download" CTAs → `brand`.

- [ ] **Step 2: Verify.** `scripts/check-off-token-colors.sh | grep -E 'ChunkProgress|ModelManager|ModelSettingsModal|SettingsModal|LanguageSelection|DeviceSelection|BluetoothPlayback|BetaSettings|RecordingSettings|TranscriptSettings|CalendarSettings|AudioBackendSelector|TemplateSettings|ComplianceNotification'` → no output. Gate: `pnpm lint && pnpm test && npx tsc --noEmit` (`CalendarSettings.test.tsx`, `RecordingSettings.test.tsx`, `LanguageSelection.test.tsx` all exercise these files). Launch: every Settings tab in both themes.

- [ ] **Step 3: Commit** — `git commit -am "refactor(0057): sweep settings and model managers to semantic tokens"`

### Task 13: Sweep — onboarding, permissions, import, dialogs; gate goes green

**Files:**
- Modify: `frontend/src/components/onboarding/shared/PermissionRow.tsx`, `onboarding/shared/StatusIndicator.tsx`, `onboarding/shared/ProgressIndicator.tsx`, `onboarding/steps/DownloadProgressStep.tsx`, `onboarding/steps/PermissionsStep.tsx`, `PermissionWarning.tsx`, `shared/DownloadProgressToast.tsx`, `ImportAudio/ImportAudioDialog.tsx`, `ImportAudio/ImportDropOverlay.tsx`, `DatabaseImport/HomebrewDatabaseDetector.tsx`, `DatabaseImport/LegacyDatabaseImport.tsx`, `People/VoiceprintSamplesList.tsx`

- [ ] **Step 1: Apply the mapping table.** `StatusIndicator.tsx:12-17` (`green-600`/`red-300`/`yellow-400`) → `success` / `destructive` / `brand`; `ProgressIndicator.tsx` active step → `bg-brand`, done → `bg-success`, pending → `bg-border`. `ImportDropOverlay.tsx` full-screen tint: `bg-brand/10` with a `border-brand` dashed frame.

- [ ] **Step 2: Verify the gate is fully green**

Run: `scripts/check-off-token-colors.sh`
Expected: `check-off-token-colors: ok`. If any hit remains and is a deliberate exception (there should be none in this plan), it needs a `token-gate: allow` comment with a reason on that line.

- [ ] **Step 3: Full gate + onboarding smoke**

Run: `cd frontend && pnpm lint && pnpm test 2>&1 | tail -4 && npx tsc --noEmit && cd .. && scripts/check-file-size.sh`. Then launch with a fresh dev profile to see onboarding (`frontend/clean_run.sh` — **read its header first**; per specs/0052 it once wrote production data; confirm it targets `ai.vinyl.app.debug` before running), walk the four onboarding steps in Deck, then in Faceplate.

- [ ] **Step 4: Commit** — `git commit -am "refactor(0057): sweep onboarding/import/dialog surfaces; off-token gate green"`

---

### Task 14: CI gate, DoD run, handover

**Files:**
- Modify: `.github/workflows/ci-checks.yml` (frontend job), `CLAUDE.md` (Definition of Done list)

- [ ] **Step 1: Add the gate to CI**

In `.github/workflows/ci-checks.yml`, in the frontend job after the `Unit tests` step:
```yaml
      - name: Semantic-token gate (specs/0057)
        run: ../scripts/check-off-token-colors.sh
```
(check the job's `working-directory`; the existing `pnpm` steps run in `frontend`, so the script path is `../scripts/…`. If the job runs at the root, use `scripts/check-off-token-colors.sh`.)

- [ ] **Step 2: Add it to the Definition of Done**

`CLAUDE.md` "Definition of Done" — add after item 2b:
```
2c. `scripts/check-off-token-colors.sh` passes (specs/0057: no raw Tailwind palette classes —
   everything goes through the semantic tokens so Deck/Faceplate stay in parity).
```

- [ ] **Step 3: Run the full Definition of Done**

```bash
cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo clippy --features metal --all-targets -- -D warnings && cargo test --features metal 2>&1 | tail -3
cd ../ && pnpm lint && pnpm test 2>&1 | tail -3 && npx tsc --noEmit
cd .. && scripts/check-file-size.sh && scripts/check-off-token-colors.sh
```
Expected: all clean. (The `vad_filter` adversarial-fixtures test is a known pre-existing failure on macOS 15.x — see memory; report it, do not chase it.)

- [ ] **Step 4: Owner smoke (cannot be done by the agent — hand over with this checklist)**

1. `frontend/dev-nixon.sh` → window "Dev Nixon", new Dock icon.
2. Settings → General → Appearance: Faceplate / Deck / System each apply instantly; relaunch keeps the choice; System follows a macOS appearance flip.
3. Record → live transcript → stop → summary, once in each theme; the summary editor text is readable in Deck.
4. Every Settings tab, People, Today, All meetings, one meeting page, onboarding (fresh dev profile) — no white-on-white or invisible chips in Deck.
5. `.env.signing` updated to `NIXON_GOOGLE_CLIENT_ID` / `NIXON_GOOGLE_CLIENT_SECRET` before the next `./release.sh`.

- [ ] **Step 5: Commit and push the branch**

```bash
git add .github/workflows/ci-checks.yml CLAUDE.md
git commit -m "ci(0057): enforce the semantic-token gate; DoD updated"
git push -u origin feat/0057-nixon-rebrand
```
Do **not** merge or release: Plans 2 and 3 land on this same branch first (spec decision 12).

---

## Self-review notes

- **Spec coverage (Phase A):** rename surfaces from spec §7 Phase A table — `APP_NAME` (T2), productName/title (T2), npm/crate/binary + `cargo-dev-sign.sh`/`dev`/`build`/`upgrade`/`clean_run`/`release`/`tauri-auto.js` (T2, T3), metadata/Sidebar/About/product copy (T4), icons (T6), docs + changelog (T5), the 3 test assertions (T4). Decision 11's "scrub visible references" extends this to internal keys (T4) and recordings folder / env vars (T2). Tray template icon by state is Phase C (Plan 2) — not here.
- **Spec coverage (Phase B):** token blocks (T7), fonts (T7), `tailwind.config.js` additions incl. `success`/`panel`/`engrave`/`meter-over` and dropping `tertiary` (T7), `--radius` (T7), `.u-*` rewrite (T7), keyframes + scrollbar hex removed (T7), theme default + selector (T8, T9), `theme="light"` fixed in **both** BlockNote hosts (T8), the 342-hit sweep incl. `speaker-colors.ts`, `button.tsx` green, `DevBadge`, `AudioLevelMeter`, `TimelineBlock`, `people/page.tsx` star, onboarding indicators (T10–T13), gate in CI (T1, T14). `.summary-doc` dark background: the `transparent` editor background already inherits `--card` in both themes once T7 lands; verified by eye in T8 step 6.
- **Deliberately deferred to Plan 2/3:** `rounded-full` pills, button variant renames `blue→brand`/`red→record`, unifying the three page-header formulas, the framer-motion tab underline, `Switch` restyle, tray icons, `RecordingControls` dead `showPlayback` branch (T10 only deletes the *unmounted* `RecordingStatusBar`).
- **Type consistency:** `useTheme()` returns `{ preference, resolved, setPreference }` in T8 and is consumed with those names in T8 step 5 and T9. `THEME_STORAGE_KEY = 'nixon.theme'` in T8 and T9 tests. Speaker-color exports are asserted by class set, with an explicit instruction to read the real export names.
