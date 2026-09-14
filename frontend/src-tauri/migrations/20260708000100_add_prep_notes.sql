-- specs/0036 (pre-call prep): "prep notes" — the things you plan to COVER in a meeting,
-- written before it starts. Stored on the existing one-row-per-meeting `meeting_notes` table
-- (mirrors how the dormant `enhanced_*` columns were added), but in DEDICATED columns kept
-- separate from `notes_markdown` (the live during-meeting notes) so:
--   1. prep agenda is not silently mixed into your live notes, and
--   2. we can choose independently whether/how prep feeds the post-meeting summary (it is
--      passed as a labeled "intended agenda" block, NOT as live notes, and is NOT fed to the
--      action-item extractor).
-- Nullable/additive; existing rows are unaffected.
ALTER TABLE meeting_notes ADD COLUMN prep_markdown TEXT;
ALTER TABLE meeting_notes ADD COLUMN prep_json TEXT;
ALTER TABLE meeting_notes ADD COLUMN prep_updated_at TEXT;
