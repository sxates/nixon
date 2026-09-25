-- specs/0078: who was on the mic, per meeting.
--
-- `audio_setup` is the user's override: NULL = detect automatically, 'room' = everyone
-- was in the room on one shared mic, 'call' = just the owner on the mic, others on the
-- call. `audio_setup_resolved` is what the last diarization pass actually used:
-- NULL (no pass since this migration) | 'call' | 'room' | 'hybrid'. It is written in the
-- same transaction that persists the pass's speaker keys, so a failed pass never leaves
-- it out of step with the rows alignment, the suggestion refetch, and the UI read.
--
-- `owner_label` is what the user said about "You" in this meeting: NULL = nothing (any
-- "You" is automatic), 'confirmed' = "This is me" (cleared again when a pass rebuilds
-- the rows), 'rejected' = "This isn't me" (sticky: the owner's voiceprint and the
-- previous "You" no longer auto-label a cluster in this meeting; "This is me" clears it).
--
-- Forward-only (CLAUDE.md): never edit an applied migration. NULL in every column means
-- today's behavior, so no backfill.
ALTER TABLE meetings ADD COLUMN audio_setup TEXT;
ALTER TABLE meetings ADD COLUMN audio_setup_resolved TEXT;
ALTER TABLE meetings ADD COLUMN owner_label TEXT;
