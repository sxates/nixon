-- specs/0079 W1: saved summary prompts replace summary templates.
-- Template conversion happens in Rust at startup (summary/prompt_migration.rs); the old
-- meeting_summary_outlines table is dropped later by 20261003000100 (after W5 removes its readers).

CREATE TABLE IF NOT EXISTS summary_prompts (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    body TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_default INTEGER NOT NULL DEFAULT 0,
    extract_action_items INTEGER NOT NULL DEFAULT 1,
    in_library INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_summary_prompts_single_default
    ON summary_prompts(is_default) WHERE is_default = 1;
CREATE UNIQUE INDEX IF NOT EXISTS idx_summary_prompts_name_nocase
    ON summary_prompts(name COLLATE NOCASE) WHERE in_library = 1;

CREATE TABLE IF NOT EXISTS series_summary_prompts (
    series_key TEXT PRIMARY KEY NOT NULL,
    prompt_id TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS summary_prompts_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

ALTER TABLE meetings ADD COLUMN summary_prompt_id TEXT;
ALTER TABLE meetings ADD COLUMN custom_summary_prompt TEXT;
ALTER TABLE meetings ADD COLUMN custom_extract_action_items INTEGER;
