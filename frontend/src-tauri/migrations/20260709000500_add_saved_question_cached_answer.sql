-- specs/0038 dogfood feedback #4: a saved question now has its own sub-page that shows the
-- CACHED answer from the last time it ran (no auto-run on open) and only regenerates when the
-- user presses Rerun. Cache the last answer's markdown + sources on the row so the sub-page can
-- render instantly without an LLM call. The existing `last_run_at` column (previously only the
-- `touch_last_run` seam) becomes the real "answered at" timestamp, stamped by `update_answer`.
--
-- Forward-only (CLAUDE.md): never edit an applied migration. SQLite allows only one column per
-- ALTER TABLE, so each new column is its own statement. Columns are nullable (NULL = "never run
-- yet"), so no backfill is needed for existing rows.
ALTER TABLE saved_questions ADD COLUMN last_answer_markdown TEXT;
ALTER TABLE saved_questions ADD COLUMN last_sources_json TEXT;
