-- specs/0038 WS2 (Ask-AI history & saved questions): a thin persistence layer over the 0035
-- Ask-AI engine. `execute_ask_ai` stays ephemeral; a fire-and-forget hook records each
-- COMPLETED run into `ask_ai_history` (WS2.a), and the user may "star" a question + its scope
-- into `saved_questions` to re-run against fresh data later (WS2.b). No embeddings, no
-- re-answer on load — re-run simply re-invokes `api_ask_ai_run` with the stored question+scope.
--
-- Forward-only (CLAUDE.md): never edit an applied migration. Idempotent (CREATE ... IF NOT
-- EXISTS); sqlx also runs each migration at most once (_sqlx_migrations).

-- One row per COMPLETED Ask-AI run (WS2.a). NOT keyed to a single meeting: a run spans a
-- SCOPE of meetings, and the answer's cited/uncited sources live in `sources_json`. `scope_json`
-- is the exact serialized `AggregationScope` the run used, so a re-run from history round-trips
-- the scope verbatim.
CREATE TABLE IF NOT EXISTS ask_ai_history (
    id              TEXT PRIMARY KEY,            -- "aah-<uuid>"
    question        TEXT NOT NULL,
    scope_json      TEXT NOT NULL,               -- serialized AggregationScope (camelCase)
    answer_markdown TEXT NOT NULL,               -- [M#]-post-processed answer markdown
    sources_json    TEXT NOT NULL,               -- JSON array of SourceMeeting
    provider        TEXT,                        -- summary provider the run followed (nullable)
    model           TEXT,                        -- model the run followed (nullable)
    created_at      TEXT NOT NULL                -- ISO-8601 UTC
);
-- The history list is most-recent-first; index the sort key.
CREATE INDEX IF NOT EXISTS idx_ask_ai_history_created ON ask_ai_history(created_at DESC);

-- Reusable "starred" question (WS2.b): a label + question text + scope the user can re-run at
-- any time. Re-run re-invokes `api_ask_ai_run` with (question, scope_json), so it re-gathers and
-- re-answers against whatever data now exists ("key decisions today" evaluates today, every
-- time). `last_run_at` is the seam a future scheduled/digest feature would build on.
CREATE TABLE IF NOT EXISTS saved_questions (
    id          TEXT PRIMARY KEY,                -- "saq-<uuid>"
    label       TEXT NOT NULL,                   -- user-facing name for the saved question
    question    TEXT NOT NULL,
    scope_json  TEXT NOT NULL,                   -- serialized AggregationScope (camelCase)
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    last_run_at TEXT                             -- NULL until first re-run; touch_last_run stamps it
);
CREATE INDEX IF NOT EXISTS idx_saved_questions_created ON saved_questions(created_at DESC);
