-- Migration: structured action items extracted from meeting summaries + the per-meeting
-- extraction ledger (specs/0034 Action items v1).
--
-- `action_items` is the SOURCE OF TRUTH for task state (the summary markdown stays a
-- document). Rows are either machine-extracted from the generated summary (`source =
-- 'extracted'`) or hand-added (`source = 'manual'`, optionally standalone with a NULL
-- meeting_id). Re-extraction after a summary regeneration runs a DIFF (src/action_items/
-- diff.rs), never a wipe-and-reload: protected rows (manual, user-edited, or non-open
-- status) are never modified, deleted, or duplicated by a re-run; only machine-owned
-- PRISTINE rows (extracted + open + untouched) may be rewritten.
--
-- Assignee shape (deviates deliberately from the 0013 Primitive-2 sketch): the app owner is
-- NOT a `people` row (the roster excludes self, migrations/20260629000000), so "assigned to
-- me" is its own flag; unresolved names keep `assignee_raw` for display.
--
-- FKs below are DOCUMENTATION ONLY (no declared constraints); cascades are enforced
-- explicitly in the delete transactions (see repositories/meeting.rs — meeting delete drops
-- this meeting's items + ledger row — and repositories/people.rs — person delete NULLs
-- `assignee_person_id`, the item outlives the person).
--
-- Forward-only (CLAUDE.md): never edit an applied migration. Idempotent (CREATE ... IF NOT
-- EXISTS); sqlx also runs each migration at most once (_sqlx_migrations).
CREATE TABLE IF NOT EXISTS action_items (
    id                 TEXT PRIMARY KEY,            -- "ai-<uuid>"
    meeting_id         TEXT,                        -- → meetings.id; NULL = standalone manual to-do
    description        TEXT NOT NULL,
    assignee_person_id TEXT,                        -- → people.id; NULL = me / unresolved / unassigned
    assignee_is_self   INTEGER NOT NULL DEFAULT 0,  -- 1 = the app owner ("me"); owner is NOT a people row
                                                    -- (roster excludes self, migrations/20260629000000)
    assignee_raw       TEXT,                        -- name as extracted when unresolved (display fallback)
    due_hint           TEXT,                        -- verbatim extracted hint ("Friday", "2026-07-10"); no parsing in v1
    status             TEXT NOT NULL DEFAULT 'open',-- 'open' | 'completed' | 'dismissed'
    source             TEXT NOT NULL DEFAULT 'extracted', -- 'extracted' | 'manual'
    user_edited        INTEGER NOT NULL DEFAULT 0,  -- 1 = user has touched this row (edited content OR
                                                    -- changed status, incl. back to open) → protected
    content_key        TEXT NOT NULL,               -- fingerprint of normalized description (diff identity)
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    completed_at       TEXT                         -- set when status → 'completed'
);
CREATE INDEX IF NOT EXISTS idx_action_items_meeting  ON action_items(meeting_id);
CREATE INDEX IF NOT EXISTS idx_action_items_assignee ON action_items(assignee_person_id);
CREATE INDEX IF NOT EXISTS idx_action_items_status   ON action_items(status);
-- DB backstop for the diff engine's no-duplicates invariant: one EXTRACTED row per
-- (meeting, content_key). The diff already dedupes candidates by content_key and drops
-- candidates absorbed by protected rows (src/action_items/diff.rs), and run_extraction
-- serializes runs per meeting (in-flight guard) — this index turns any future regression
-- into a loud constraint error instead of silent duplicate tasks. Manual rows are
-- exempt: the user may hand-add a to-do that shadows an extracted one.
CREATE UNIQUE INDEX IF NOT EXISTS idx_action_items_extracted_content
    ON action_items(meeting_id, content_key) WHERE source = 'extracted';

-- One row per meeting: the extraction ledger (idempotence + debuggability). Re-running
-- extraction when `summary_fingerprint` matches the current summary+notes is a no-op;
-- failed extractions never write here, so "Scan again" retries after LLM flakiness.
CREATE TABLE IF NOT EXISTS action_item_extractions (
    meeting_id          TEXT PRIMARY KEY,           -- → meetings.id
    summary_fingerprint TEXT NOT NULL,              -- stable_text_fingerprint(summary markdown + notes)
    extracted_at        TEXT NOT NULL,
    model_provider      TEXT NOT NULL,
    model_name          TEXT NOT NULL,
    item_count          INTEGER NOT NULL
);
