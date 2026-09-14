-- specs/0036 (pre-call prep): persist the calendar SERIES key onto the meeting row so
-- recurring occurrences can be grouped into a series across both calendar sources.
--
-- The key is EventKit's `calendarItemExternalIdentifier` / Google's iCalUID (surfaced as
-- `external_id` on the calendar DTOs). Unlike `calendar_event_id` — which is shared across a
-- recurring series on EventKit but UNIQUE per occurrence on Google (singleEvents=true) —
-- `external_id` is series-level for BOTH sources, so it is the one reliable cross-source
-- grouping key. Series detection is: match `calendar_series_key` when present, else fall back
-- to normalized-title matching (the specs/0020 approach) for manual/older/notes-only rows.
--
-- Nullable: pre-existing, ad-hoc, and notes-only meetings stay NULL (no backfill; they fall
-- back to title match). NOT UNIQUE — a series key recurs across every occurrence, exactly like
-- `calendar_event_id`.
ALTER TABLE meetings ADD COLUMN calendar_series_key TEXT;

CREATE INDEX IF NOT EXISTS idx_meetings_calendar_series_key
  ON meetings (calendar_series_key)
  WHERE calendar_series_key IS NOT NULL;
