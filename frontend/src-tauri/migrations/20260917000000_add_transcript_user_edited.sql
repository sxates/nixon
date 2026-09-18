-- specs/0061 W5: mark a transcript segment as user-edited (correcting a
-- machine-transcription error) so the Enhance dialog can warn before a
-- regenerate/retranscribe would silently discard the correction. Defaults to
-- 0 for every existing row (nothing has been user-edited yet).
ALTER TABLE transcripts ADD COLUMN user_edited INTEGER NOT NULL DEFAULT 0;
