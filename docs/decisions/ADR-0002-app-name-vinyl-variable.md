# ADR-0002 — App name "Vinyl" is a variable

- **Status:** Accepted
- **Date:** 2026-06-23

## Context
We need a working name for the product (repo, docs, app bundle, app-data dir), but the name
is not finalized and may change.

## Decision
Use **"Vinyl"** as the current value of a single logical `APP_NAME` variable. In our own docs
and config, keep the name trivially find-and-replaceable. Defer the actual Tauri bundle /
app-data-dir rebrand to a dedicated, tested task (`specs/0002-rebrand-to-vinyl.md`) rather
than doing it ad hoc.

## Consequences
- No premature, scattered hardcoding of the name; one rebrand task owns the change.
- The app continues to build/run as "Meetily" until 0002 lands, keeping the Phase 0 baseline
  unambiguous.
- Renaming the app-data dir later requires a one-time data migration (covered in 0002).
