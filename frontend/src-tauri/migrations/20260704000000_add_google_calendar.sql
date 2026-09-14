-- Migration: Google Calendar provider state + local event cache (specs/0032, ADR-0010).
--
-- Three tables back the opt-in, read-only Google Calendar connection:
--   * google_calendar_account — the single connected account (v1 = one account).
--   * google_calendar_sync    — per-calendar sync bookkeeping (selection + syncToken).
--   * google_calendar_events  — the local event cache the agenda/upcoming/merge layers read,
--     bounded by the sync window (now-14d .. now+60d).
--
-- OAuth TOKENS ARE NOT HERE and must never be (ADR-0010): the refresh token lives in the
-- macOS Keychain (secrets.rs account 'gcal.refresh_token'); access tokens are memory-only.
-- Disconnect purges all three tables (dismissed_calendar_events is kept — its keys are
-- cross-source by design, 0029 WS6.2).
--
-- Forward-only (CLAUDE.md): never edit an applied migration. Idempotent (CREATE ... IF NOT
-- EXISTS); sqlx also runs each migration at most once (_sqlx_migrations).
CREATE TABLE IF NOT EXISTS google_calendar_account (      -- single row (v1: one account)
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    email         TEXT NOT NULL,              -- shown in Settings; from calendarList 'primary'
    connected_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS google_calendar_sync (         -- one row per calendar on the account
    calendar_id    TEXT PRIMARY KEY,
    summary        TEXT NOT NULL,             -- calendar display name
    selected       INTEGER NOT NULL DEFAULT 1,-- user's per-calendar sync toggle
    sync_token     TEXT,                      -- NULL => full (re)sync needed
    last_synced_at TEXT
);

CREATE TABLE IF NOT EXISTS google_calendar_events (
    id              TEXT PRIMARY KEY,         -- 'gcal:<calendarId>/<instanceId>'
    calendar_id     TEXT NOT NULL,
    ical_uid        TEXT,                     -- cross-source external identity
    title           TEXT NOT NULL,
    starts_at       TEXT NOT NULL,            -- RFC3339 UTC (chrono to_rfc3339)
    ends_at         TEXT NOT NULL,
    is_all_day      INTEGER NOT NULL DEFAULT 0,
    location        TEXT,
    zoom_url        TEXT,                     -- via calendar::zoom_link::extract_zoom_url
    organizer_email TEXT,
    my_response     TEXT,                     -- needsAction|declined|tentative|accepted
    attendees_json  TEXT NOT NULL DEFAULT '[]', -- [{name,email,isCurrentUser,responseStatus,isOrganizer}]
    status          TEXT NOT NULL DEFAULT 'confirmed',
    updated_at      TEXT NOT NULL
);

-- Window queries (agenda/upcoming) scan by start instant; merge-layer dedupe looks up by
-- the cross-source iCalUID.
CREATE INDEX IF NOT EXISTS idx_gcal_events_starts_at ON google_calendar_events(starts_at);
CREATE INDEX IF NOT EXISTS idx_gcal_events_ical_uid  ON google_calendar_events(ical_uid);
