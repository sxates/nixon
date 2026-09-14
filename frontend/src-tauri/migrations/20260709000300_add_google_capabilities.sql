-- Migration: Google account best-effort enrichment capabilities (specs/0038 WS3, ADR-0010
-- amendment). A granted OAuth scope is NOT access — org policy can still return 403
-- PERMISSION_DENIED — so we probe once at connect time and cache what the account can
-- actually do. NULL on every column = not yet probed (behave exactly as calendar-only, zero
-- extra egress).
--
--   * can_expand_groups   — Cloud Identity group-member listing works (flatten DL invites).
--   * can_fetch_photos    — People-API directory read works (attendee photos; sibling agent).
--   * capabilities_probed_at — RFC3339 UTC of the last probe (re-probe only when stale).
--
-- Additive + idempotent (CLAUDE.md forward-only rule): ALTER TABLE ADD COLUMN is a no-op-safe
-- one-time change; sqlx runs each migration at most once. Columns are nullable with no default
-- so existing single-row accounts read back NULL (= unprobed) until the next connect/probe.
ALTER TABLE google_calendar_account ADD COLUMN can_expand_groups INTEGER;
ALTER TABLE google_calendar_account ADD COLUMN can_fetch_photos INTEGER;
ALTER TABLE google_calendar_account ADD COLUMN capabilities_probed_at TEXT;
