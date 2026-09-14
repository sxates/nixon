-- Migration: link a per-meeting speaker occurrence to the durable Person
-- (specs/0016 Phase 1b).
--
-- `speakers.person_id` → `people.id`, nullable. No FK enforcement: the pool connects
-- WITHOUT `PRAGMA foreign_keys = ON`, so cascades never fire — the cleanup is enforced
-- explicitly in delete transactions (see PeopleRepository::delete, which NULLs this
-- column for the deleted person; and meeting delete, which simply drops the speaker
-- rows so People outlive their meetings, by design).
--
-- Forward-only (CLAUDE.md): never edit a prior migration. SQLite has no
-- `ADD COLUMN IF NOT EXISTS`, but sqlx runs each migration at most once (tracked in
-- _sqlx_migrations), so a plain additive ADD COLUMN is idempotent in practice.
-- Nullable with no default so existing rows are unaffected.
ALTER TABLE speakers ADD COLUMN person_id TEXT;  -- → people.id, nullable

-- Partial index: most speaker rows have a NULL person_id, so index only the linked
-- ones (keeps the index small; the matcher / delete paths query by person_id).
CREATE INDEX IF NOT EXISTS idx_speakers_person_id ON speakers(person_id) WHERE person_id IS NOT NULL;
