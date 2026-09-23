-- specs/0072 W1: where each meeting's audio is in its life, written by Rust
-- (audio/lifecycle). NULL = pending (not processed yet) | 'processed' | 'failed'
-- (speaker identification errored) | 'purged' (the retention policy deleted the audio).
ALTER TABLE meetings ADD COLUMN audio_state TEXT;

-- The last successful offline speaker identification. Cleared when the transcript
-- rows are replaced (retranscription) or a resumed recording adds a segment.
ALTER TABLE meetings ADD COLUMN speakers_identified_at TEXT;

-- Backfill: every existing meeting with a folder that is NOT awaiting transcription is
-- 'processed'. "Awaiting" is the pre-0072 backlog predicate (explicit 'defer', or a sparse
-- transcript with no completed summary) plus a stranded 'live' marker, which the startup
-- reconcile turns back into 'defer'. Those rows stay NULL so the backlog still sees them.
UPDATE meetings SET audio_state = 'processed'
 WHERE folder_path IS NOT NULL
   AND COALESCE(processing_mode, '') NOT IN ('defer', 'live')
   AND NOT ((SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = meetings.id) < 3
            AND NOT EXISTS (SELECT 1 FROM summary_processes sp
                             WHERE sp.meeting_id = meetings.id AND sp.status = 'completed'));

UPDATE meetings SET speakers_identified_at = updated_at
 WHERE EXISTS (SELECT 1 FROM speakers s WHERE s.meeting_id = meetings.id);
