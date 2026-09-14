-- Per-meeting processing-mode override (low-power-mode spec §3). NULL — the value
-- for all existing and new rows — means "follow the global low-power/battery
-- decision". 'live' forces full live processing (set when the user flips a
-- deferred meeting live, incl. mid-recording); 'defer' forces record-only and
-- marks the meeting pending for backlog processing. Cleared back to NULL when
-- backlog/stop-time processing completes.
ALTER TABLE meetings ADD COLUMN processing_mode TEXT;
