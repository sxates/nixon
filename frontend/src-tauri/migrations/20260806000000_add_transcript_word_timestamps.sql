-- specs/0046 WS2 Task 6: per-word timestamps for the batch transcription path.
--
-- Stores the Parakeet batch engine's per-word `{text, start, end}` stamps as a
-- JSON array (`[{"w":..,"s":..,"e":..}, ..]`) so the offline diarization split
-- (Task 7) can snap speaker-turn boundaries to word edges instead of arbitrary
-- sample positions. NULL for legacy rows, Whisper-transcribed rows (no
-- per-word timing available), and any source with no per-channel capture.
ALTER TABLE transcripts ADD COLUMN word_timestamps TEXT;
