-- The default summary prompt preset was first seeded as "Let the model choose the structure"
-- and renamed to "Auto" (prompt_migration.rs AUTO_PRESET_NAME) days later. Libraries converted
-- before that rename kept the long name, which truncates in the record screen's prompt picker.
-- Rename only that exact seeded name, and only when no prompt is already called "Auto"
-- (names are unique, case-insensitively). A prompt the owner renamed is left alone.
UPDATE summary_prompts
SET name = 'Auto'
WHERE name = 'Let the model choose the structure'
  AND NOT EXISTS (SELECT 1 FROM summary_prompts WHERE name = 'Auto' COLLATE NOCASE);
