-- Migration: sticky per-segment speaker corrections (specs/0019 WS2.3, note 8).
--
-- Diarization can put a single transcript line under the wrong speaker. The user can
-- reassign one line to another speaker — but the offline diarization pass
-- (pipeline::persist) NULLs every `transcripts.speaker` and rebuilds the `speakers`
-- rows from scratch, which would silently wipe that manual fix. This table records the
-- correction durably, keyed by the STABLE `transcripts.id` (segments aren't reinserted
-- across re-runs), so `persist` can re-apply it after each re-diarization.
--
-- One override per transcript line (PK = transcript_id); reassigning the same line again
-- just overwrites it (INSERT ... ON CONFLICT). `speaker_key` is the target speaker within
-- the meeting (e.g. 'local', 'spk_2'); deleting the row reverts the line to whatever the
-- diarizer assigns.
--
-- FK is documentation-only (no PRAGMA foreign_keys on the pool, like the rest of the
-- schema); the meeting-delete cascade is enforced explicitly in the delete transaction.
-- Forward-only (CLAUDE.md): never edit an applied migration. Idempotent.
CREATE TABLE IF NOT EXISTS transcript_speaker_overrides (
    meeting_id    TEXT NOT NULL,                 -- → meetings.id    (FK doc-only)
    transcript_id TEXT NOT NULL,                 -- → transcripts.id (FK doc-only)
    speaker_key   TEXT NOT NULL,                 -- target speaker within the meeting
    created_at    TEXT NOT NULL,
    PRIMARY KEY (transcript_id)
);

-- Re-apply on re-diarization reads all overrides for a meeting; index that path.
CREATE INDEX IF NOT EXISTS idx_transcript_overrides_meeting
    ON transcript_speaker_overrides(meeting_id);
