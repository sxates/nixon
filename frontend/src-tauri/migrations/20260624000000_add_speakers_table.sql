-- Migration: speaker-diarization speakers table (specs/0010, ADR-0005, P1-B2)
--
-- Maps a per-meeting speaker KEY (the value written into transcripts.speaker:
-- "local" for the mic/local user, "spk_0","spk_1",… for clustered remote speakers)
-- to an editable DISPLAY NAME ("You","Speaker 1", or a user rename like "Priya").
--
-- A meeting has few speakers but many transcript segments, so renaming a speaker is
-- one UPDATE here rather than N updates across transcripts; the reserved `embedding`
-- BLOB gives P3 cross-meeting identity a home without another migration.
--
-- Forward-only (CLAUDE.md). Idempotent: CREATE TABLE IF NOT EXISTS. The meeting_id
-- FK declares ON DELETE CASCADE, but the app pool connects without
-- `PRAGMA foreign_keys = ON`, so deletes also clear speakers explicitly in the
-- meeting-delete transaction (see repositories/meeting.rs).
CREATE TABLE IF NOT EXISTS speakers (
    id           TEXT PRIMARY KEY,            -- "speaker-<uuid>"
    meeting_id   TEXT NOT NULL,
    speaker_key  TEXT NOT NULL,               -- matches transcripts.speaker ("local","spk_0",…)
    display_name TEXT NOT NULL,               -- "You","Speaker 1", user-renamed e.g. "Priya"
    is_local     INTEGER NOT NULL DEFAULT 0,  -- 1 for the mic/local user
    embedding    BLOB,                        -- reserved for P3 cross-meeting identity (nullable)
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    UNIQUE(meeting_id, speaker_key),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

-- Lookups are always scoped to a meeting (get_by_meeting, the transcript join).
CREATE INDEX IF NOT EXISTS idx_speakers_meeting_id ON speakers(meeting_id);
