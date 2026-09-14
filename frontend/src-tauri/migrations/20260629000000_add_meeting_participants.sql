-- Migration: the durable, editable participant roster for a meeting (specs/0017 Phase A).
--
-- `meeting_participants` is distinct from `speakers` (who actually SPOKE). Each row links a
-- meeting to an app-wide Person (people.id) — the invited / known roster. Because a
-- participant IS a `people` row, auto-People and role/notes come for free (specs/0016).
-- `source` records how the row got here: 'calendar' (seeded from the linked EventKit event)
-- or 'manual' (hand-added on a meeting screen).
--
-- The owner (`is_current_user`) is EXCLUDED from the roster by the seeding code: the roster
-- is the OTHER people; "You" is implicit (the mic channel), consistent with diarization's
-- exclude-self. So a participant_id here is always a remote person.
--
-- No `PRAGMA foreign_keys` on the pool → the declared FKs below are DOCUMENTATION ONLY; the
-- meeting-delete and person-delete cascades are enforced explicitly in their delete
-- transactions (see repositories/meeting.rs and repositories/people.rs).
--
-- Forward-only (CLAUDE.md): never edit an applied migration. Idempotent (CREATE ... IF NOT
-- EXISTS); sqlx also runs each migration at most once (_sqlx_migrations).
CREATE TABLE IF NOT EXISTS meeting_participants (
    meeting_id  TEXT NOT NULL,                  -- → meetings.id (FK doc-only, no PRAGMA)
    person_id   TEXT NOT NULL,                  -- → people.id   (FK doc-only, no PRAGMA)
    source      TEXT NOT NULL DEFAULT 'manual', -- 'calendar' (seeded) | 'manual' (added)
    created_at  TEXT NOT NULL,
    PRIMARY KEY (meeting_id, person_id)         -- idempotent seeding: INSERT OR IGNORE
);

-- Per-meeting roster reads (the common access path) hit the PK prefix already; index the
-- person side so person-delete's explicit cascade (DELETE ... WHERE person_id = ?) is cheap.
CREATE INDEX IF NOT EXISTS idx_meeting_participants_meeting ON meeting_participants(meeting_id);
CREATE INDEX IF NOT EXISTS idx_meeting_participants_person ON meeting_participants(person_id);
