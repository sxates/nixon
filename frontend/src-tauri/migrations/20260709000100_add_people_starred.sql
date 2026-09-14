-- Add a `starred` priority flag to `people` (specs/0038 WS5.a).
--
-- Until now `people` had no priority signal — pick-lists sorted purely alphabetically,
-- so a user's top collaborators were buried in an unranked wall. `starred` lets a person
-- be pinned to the top of ranked pick-lists (starred first, then by call frequency).
--
-- Stored as a SQLite integer (0/1); exposed to the frontend as a bool. Forward-only,
-- additive, idempotent guard via the DEFAULT so existing rows backfill to 0 (unstarred).

ALTER TABLE people ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;
