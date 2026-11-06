-- specs/0084 W1: the speaker-identification crash-loop breaker.
-- Set while an offline speaker-identification pass runs, cleared when it ends (success,
-- error, or a graceful quit). Still set at launch = the app died mid-pass.
ALTER TABLE meetings ADD COLUMN speaker_id_started_at TEXT;
-- Why the last pass failed: interrupted | crashed | cancelled | error (NULL = no failure).
ALTER TABLE meetings ADD COLUMN speaker_id_failure TEXT;
-- Passes that died mid-run. >= 1 makes the next (user-started) pass run in safe mode;
-- only a successful pass resets it.
ALTER TABLE meetings ADD COLUMN speaker_id_crashes INTEGER NOT NULL DEFAULT 0;
