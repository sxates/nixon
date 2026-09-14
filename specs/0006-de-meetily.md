# 0006 — De-meetily: make it our app

- **Status:** In progress (2026-06-24)
- **Owner agent(s):** frontend-engineer + rust-core-engineer
- **Roadmap phase:** Phase 1

## Context / Problem
Vinyl is a hard fork of meetily. The product still carries meetily / Zackriya Solutions
branding and references throughout the UI and code: an About page crediting them, links to
their website/GitHub, an auto-update checker pointing at their release feed, and the window
title showing "meetily". From here forward this is **our** app and will diverge significantly,
so those references should go.

## Goals
- Remove user-facing meetily / Zackriya Solutions references from the app:
  - The **About page** content (their credits, logo, links) and any "Info" page links to
    their site / GitHub / socials.
  - The **auto-update checker** (UI + backend) — it points at meetily's release feed and we
    have no update server yet; remove it entirely (can be re-added later for Vinyl).
  - **External links** to meetily.zackriya.com, github.com/Zackriya-Solutions, their docs, etc.
  - The **window / document title** → "Vinyl" (currently shows "meetily").
  - Visible "meetily" / "Zackriya" **strings** in UI copy, toasts, metadata, onboarding.
- Clean up obvious meetily/Zackriya references in code comments/strings where low-risk.

## Non-goals / explicitly deferred
- **Bundle identifier + app-data dir rebrand** (`com.meetily.ai` → `com.vinyl`) and the
  **crate/npm package name** (`meetily` → vinyl): deferred to `specs/0002` — these change the
  SQLite/data location and have migration implications, and `dev` already uses `com.vinyl.dev`.
  Leave `name = "meetily"` in Cargo.toml / package.json and the identifier as-is for now.
- Do not remove the upstream attribution in `docs/upstream/` or the fork note in `CLAUDE.md` /
  `CHANGELOG.md` (we keep an honest record that we forked meetily; MIT requires license
  attribution — keep the LICENSE).
- Do not touch the audio/STT pipeline or other inherited functionality (only branding/refs).

## Approach (audit-driven)
1. Grep the repo for `meetily`, `Meetily`, `zackriya`, `Zackriya`, update-feed URLs, and their
   domains/socials. Categorize: (a) user-facing → remove/rebrand to Vinyl; (b) internal
   identifiers gated to 0002 → leave; (c) license/attribution → keep.
2. **Frontend:** delete/replace the About page content; remove `UpdateCheckProvider`,
   `UpdateDialog`, `UpdateNotification`, `useUpdateCheck` and their mounts in `layout.tsx`;
   strip external links; set the window/document title to "Vinyl"; rebrand visible strings.
   Note any Tauri update commands the frontend stops calling.
3. **Backend:** remove the update-checker command(s)/logic and the Tauri updater plugin config
   (if present); remove meetily/Zackriya references in Rust strings/comments where low-risk;
   keep the `meetily` crate name (0002).
4. Keep the LICENSE (MIT) and a short upstream-fork acknowledgement somewhere non-prominent.

## Acceptance criteria
- No user-facing meetily/Zackriya references remain in the running app (About, links, titles,
  update prompts).
- `cargo check` + `clippy` clean; `pnpm lint`/`tsc` clean for touched files; app still
  launches and the record → summary path still works.
- A grep audit documents what was removed vs intentionally kept (identifiers → 0002; license).

## Verification
Launch the app: window title reads "Vinyl"; no About/credits to Zackriya; no update prompt; no
links to their site. `git grep -i meetily | git grep -i zackriya` shows only the kept items
(crate/package name, LICENSE, upstream docs, CHANGELOG fork note) — each justified.
