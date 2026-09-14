-- Migration: durable, app-wide People entity (specs/0016 Phase 1b).
--
-- A `people` row is the cross-meeting identity anchor: the same human across many
-- meetings is one `people` row, linked from `speakers.person_id` (next migration).
-- Identity is DECOUPLED from voiceprints (ADR-0007 §2): a person can exist with NO
-- email and NO stored voice, and the per-person `voiceprint_opt_out` flag lets a known
-- person be associated with detected speakers while NEVER having their voice modeled.
-- (Phase 1c wires the actual voiceprint deletion behind this flag; 1b only stores it.)
--
-- Forward-only (CLAUDE.md): never edit a prior migration. `CREATE TABLE IF NOT EXISTS`
-- is idempotent; sqlx also runs each migration at most once (tracked in _sqlx_migrations).
CREATE TABLE IF NOT EXISTS people (
    id                 TEXT PRIMARY KEY,            -- "person-<uuid>"
    email              TEXT,                        -- stable key; NULLABLE (voice-only/manual people)
    display_name       TEXT NOT NULL,
    role               TEXT,                        -- free-text/preset (role-weighting is a follow-on)
    notes              TEXT,
    voiceprint_opt_out INTEGER NOT NULL DEFAULT 0,  -- ADR-0007 §2: 1 = never model this person's voice
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

-- Email is UNIQUE only WHEN PRESENT. SQLite treats NULLs as distinct, and a partial
-- index with `WHERE email IS NOT NULL` lets unlimited email-less people coexist while
-- still rejecting two people sharing the same email. (Do NOT put UNIQUE on the column
-- itself — that would forbid multiple NULLs on some configs and is less explicit.)
CREATE UNIQUE INDEX IF NOT EXISTS idx_people_email ON people(email) WHERE email IS NOT NULL;
