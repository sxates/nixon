-- specs/0032 task 3 — Google Calendar sync-window bookkeeping.
--
-- Google bakes the initial sync's timeMin/timeMax window into the syncToken:
-- incremental passes only ever cover that original window. To know when "now"
-- is drifting close to the window's far edge (and a full resync must re-extend
-- it), the sync engine persists the horizon (`timeMax`) it used for the last
-- FULL sync of each calendar. NULL means "never fully synced under this
-- scheme" and forces a full pass, which is also the correct behavior for rows
-- created before this migration.
ALTER TABLE google_calendar_sync ADD COLUMN window_ends_at TEXT;
