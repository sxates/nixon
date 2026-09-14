-- Migration: attendee photo cache (specs/0038 WS3, ADR-0010 best-effort amendment). The
-- sibling of DL flattening: when the org's People directory is readable (can_fetch_photos),
-- the sync path downloads same-org attendee profile photos and stores them here as base64
-- `data:` URIs, so rendering is LOCAL-ONLY (no render-time egress to Google) and Disconnect
-- purges every byte.
--
--   * email          — lowercased attendee address (the join key against wire Attendee.email).
--   * photo_data_uri — `data:image/...;base64,...` (self-contained; no external URL kept).
--   * fetched_at     — RFC3339 UTC of the download (staleness re-fetch, e.g. >30 days).
--
-- Best-effort/optional: an empty table just means initials everywhere. Not org-specific keyed,
-- so a Disconnect clears it wholesale (commands.rs). Additive + idempotent (CLAUDE.md
-- forward-only rule); sqlx runs each migration at most once.
CREATE TABLE IF NOT EXISTS attendee_photos (
    email          TEXT PRIMARY KEY,
    photo_data_uri TEXT NOT NULL,
    fetched_at     TEXT NOT NULL
);
