# ADR-0003 — Hard fork, but track upstream

- **Status:** Accepted
- **Date:** 2026-06-23

## Context
meetily is actively maintained and ships real audio/transcription bug fixes. We want to
diverge significantly (our own product direction) without losing access to those fixes.

## Decision
Hard fork with full history, kept at the project root as our own repo. Retain meetily as the
git remote **`upstream`**. We diverge freely on `main`, but periodically review upstream and
**cherry-pick** valuable fixes — primarily in audio/STT — onto feature branches.

## Consequences
- `git fetch upstream` + the `/sync-upstream` command let us audit upstream changes without
  merging.
- We do not try to stay auto-mergeable; cherry-picks are deliberate and reviewed.
- We ignore upstream branding/PRO/legacy-backend changes.
- No `origin` is configured until we create the private Vinyl repo; no pushes without an
  explicit request.
