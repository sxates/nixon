-- Migration: voiceprint gallery — best-N voice samples per Person (specs/0016 Phase 1c,
-- ADR-0007-gated).
--
-- This is the ONLY durable biometric artifact in Vinyl. A `voiceprints` row is one
-- L2-normalized CAM++ speaker embedding attached to a `people` row; the match centroid
-- is computed ON READ (mean of best-N samples, re-normalized) rather than stored, so the
-- gallery is robust to voice drift and keeps per-sample provenance for deletion/tuning
-- (ADR-0007 §4). All storage is local-only and NEVER egressed (ADR-0007 §1).
--
-- `person_id` references `people.id` with ON DELETE CASCADE declared as DOCUMENTATION
-- only: the app pool connects WITHOUT `PRAGMA foreign_keys = ON`, so the cascade never
-- fires at the DB layer. The cleanup is enforced EXPLICITLY in delete transactions
-- (PeopleRepository::delete and ::set_voiceprint_opt_out call
-- VoiceprintsRepository::delete_for_person in the same tx). A missed manual delete would
-- strand biometric data (a privacy bug), so the explicit deletes are the source of truth.
--
-- Forward-only (CLAUDE.md): never edit a prior migration. `CREATE TABLE IF NOT EXISTS`
-- is idempotent; sqlx also runs each migration at most once (tracked in _sqlx_migrations).
CREATE TABLE IF NOT EXISTS voiceprints (
    id                TEXT PRIMARY KEY,           -- "vp-<uuid>"
    person_id         TEXT NOT NULL
        REFERENCES people(id) ON DELETE CASCADE,  -- documentation; cascade enforced in app tx
    embedding         BLOB NOT NULL,              -- L2-normalized f32 LE (embedding.rs serde)
    embedding_dim     INTEGER NOT NULL,           -- f32 count in the BLOB
    embedding_model   TEXT NOT NULL,              -- matcher only compares same-model vectors
    source_meeting_id TEXT,                       -- provenance; nullable if the meeting is later deleted
    sample_quality    REAL,                       -- for best-N selection + deprioritizing weak samples
    created_at        TEXT NOT NULL
);

-- The matcher loads all samples for a person (centroid-on-read) and prune-on-insert
-- selects by person; index the lookup key.
CREATE INDEX IF NOT EXISTS idx_voiceprints_person_id ON voiceprints(person_id);
