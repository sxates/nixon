-- Migration: voiceprint quarantine + cluster back-link (specs/0039 WS3 — pollution guard).
--
-- Two additive, nullable columns on the biometric `voiceprints` gallery
-- (20260628000002_add_voiceprints.sql). Forward-only (CLAUDE.md): never edit a prior
-- migration; existing rows stay valid (both columns NULL).
--
--   quarantined_at     — soft-delete marker. NULL = live (contributes to the centroid /
--                        matcher); a non-NULL RFC3339 timestamp = quarantined: EXCLUDED from
--                        `centroid_for_person` / `all_centroids` matching but the row (and its
--                        provenance) survives, so a wrong purge is recoverable (restore = clear
--                        this column). Quarantine, not hard-delete, is the default for the WS3
--                        retraction hook and the per-sample "quarantine" control.
--
--   source_speaker_key — the diarizer cluster key (`spk_N` / `local`) this sample was enrolled
--                        from, recorded alongside the existing `source_meeting_id`. Gives a WS2
--                        span correction a handle to retract exactly the samples the
--                        corrected-away cluster contributed:
--                        `WHERE source_meeting_id = ? AND source_speaker_key = ?`.
--                        LEGACY NOTE: pre-migration samples have this NULL and so cannot be
--                        retracted by cluster — only by sample id via the per-sample UI. Accepted.
ALTER TABLE voiceprints ADD COLUMN quarantined_at     TEXT;  -- NULL = live; set = excluded from centroid/match
ALTER TABLE voiceprints ADD COLUMN source_speaker_key TEXT;  -- the spk_N cluster this sample was enrolled from

-- The retraction hook looks samples up by (source_meeting_id, source_speaker_key); partial
-- index (only rows that carry a back-link) keeps it lean and skips the legacy NULLs.
CREATE INDEX IF NOT EXISTS idx_voiceprints_source
    ON voiceprints(source_meeting_id, source_speaker_key)
    WHERE source_speaker_key IS NOT NULL;
