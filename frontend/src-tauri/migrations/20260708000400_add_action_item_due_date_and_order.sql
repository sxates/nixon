-- Migration: structured due date + manual sort order for action items
-- (specs/0038 WS1 — action-items polish; extends specs/0034).
--
-- WS1.a — `due_date`: a nullable ISO-8601 date (YYYY-MM-DD, no time) that is the
-- SORTABLE/filterable due value, set primarily by the row's date control and optionally
-- best-effort filled by the extractor when `due_hint` is unambiguously a date. It sits
-- ALONGSIDE the existing verbatim `due_hint` — the hint stays as extracted provenance
-- ("Friday"), `due_date` is the structured value. No reminders/notifications (0034 posture).
--
-- WS1.b — `sort_order`: a nullable manual-ordering key. NULL for every existing row and
-- for freshly created/extracted rows; only `api_reorder_action_items` writes dense
-- 0,1,2,… values (one transaction) when the user drags rows in the hub's Manual sort.
-- The default `list` order (meeting recency) is unchanged — sort is a client-side choice
-- over these returned fields.
--
-- Forward-only (CLAUDE.md): never edit an applied migration. SQLite has no
-- `ADD COLUMN IF NOT EXISTS`, but sqlx runs each migration at most once (_sqlx_migrations),
-- and both columns are new to `action_items`, so these ALTERs run exactly once.
ALTER TABLE action_items ADD COLUMN due_date TEXT;      -- ISO-8601 date (YYYY-MM-DD); nullable; alongside due_hint
ALTER TABLE action_items ADD COLUMN sort_order INTEGER; -- manual drag order; NULL = unordered; dense 0,1,2,… on reorder
