-- Attendee-removal tombstone (specs 2026-07-21 low-power-mode design §6): a user's
-- roster removal must survive the calendar seed-on-view (`api_get_meeting_participants`
-- re-seeds on every read). NULL — the value for all existing rows — means the
-- participant is active. Non-NULL records WHEN the user removed them; the row is kept
-- so `seed_from_attendees`' INSERT OR IGNORE can never resurrect it. Cleared back to
-- NULL when the participant is re-added manually or identified as a speaker.
ALTER TABLE meeting_participants ADD COLUMN removed_at TEXT;
