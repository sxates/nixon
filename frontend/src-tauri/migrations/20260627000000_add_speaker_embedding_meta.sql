-- Migration: tag the reserved speakers.embedding BLOB with its dimension + model
-- (specs/0016 Phase 1a, ADR-0007 §4).
--
-- The cross-meeting matcher compares CAM++ cosine vectors; comparing vectors from
-- different embedding models is meaningless. Stamping the model id (and the f32 count)
-- next to each stored embedding means a future model swap invalidates comparisons
-- cleanly rather than silently mismatching. These columns are only written for REMOTE
-- speakers on the offline pass; `local`/"You" rows keep them NULL.
--
-- Forward-only (CLAUDE.md): never edit a prior migration. SQLite has no
-- `ADD COLUMN IF NOT EXISTS`, but sqlx runs each migration at most once (tracked in
-- _sqlx_migrations), so plain additive ADD COLUMNs are idempotent in practice.
-- Nullable with no default so existing rows (all NULL embedding) are unaffected.
ALTER TABLE speakers ADD COLUMN embedding_dim   INTEGER;  -- f32 count in the embedding BLOB
ALTER TABLE speakers ADD COLUMN embedding_model TEXT;     -- e.g. "3dspeaker_campplus_sv_en_voxceleb_16k"
