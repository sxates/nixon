-- specs/0078: who was on the mic, per meeting.
--
-- `audio_setup` is the user's override: NULL = detect automatically, 'room' = everyone
-- was in the room on one shared mic, 'call' = just the owner on the mic, others on the
-- call. `audio_setup_resolved` is what the last diarization pass actually used:
-- NULL (no pass since this migration) | 'call' | 'room' | 'hybrid'. It is written at the
-- start of each pass so alignment, the suggestion refetch, and the UI read one answer.
--
-- Forward-only (CLAUDE.md): never edit an applied migration. NULL in both columns means
-- today's behavior, so no backfill.
ALTER TABLE meetings ADD COLUMN audio_setup TEXT;
ALTER TABLE meetings ADD COLUMN audio_setup_resolved TEXT;
