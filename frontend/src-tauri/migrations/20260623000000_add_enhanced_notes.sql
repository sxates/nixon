-- Note-enhancement pipeline (spec 0003): store AI-enhanced notes alongside the user's raw
-- notes. notes_markdown/notes_json remain the user's source-of-truth notes and are never
-- overwritten by enhancement; the enhanced_* columns hold the AI output.
ALTER TABLE meeting_notes ADD COLUMN enhanced_markdown TEXT;
ALTER TABLE meeting_notes ADD COLUMN enhanced_json TEXT;
ALTER TABLE meeting_notes ADD COLUMN enhanced_at TEXT;
ALTER TABLE meeting_notes ADD COLUMN enhanced_model TEXT;
