-- 2.0 Cluster 1 (specs/0015): link a recording back to its calendar event so attendee
-- lookup is exact (replaces the title+start-instant re-location in diarization/commands.rs).
-- Nullable: ad-hoc/notes-only/pre-existing meetings have no event. Not UNIQUE — the same
-- recurring event id can recur across occurrences; we don't dedupe here.
ALTER TABLE meetings ADD COLUMN calendar_event_id TEXT;
CREATE INDEX IF NOT EXISTS idx_meetings_calendar_event_id
    ON meetings(calendar_event_id) WHERE calendar_event_id IS NOT NULL;
