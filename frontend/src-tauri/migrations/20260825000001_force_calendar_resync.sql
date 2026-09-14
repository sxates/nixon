-- specs/0054 W5: before this release the incremental sync cached recurring SERIES
-- MASTERS as though they were occurrences, because `GcalEvent` never deserialized
-- `recurrence`. Those rows are keyed `gcal:<cal>/<seriesId>`, while cancellations
-- arrive keyed `<seriesId>_<instanceTs>` — so nothing could ever delete them and
-- they showed as phantom meetings on the agenda indefinitely.
--
-- On the reporter's machine 384 of 1,525 cached rows (25%) were such phantoms:
-- 245 `..._R<localStart>` masters and 139 bare series ids.
--
-- The code fix stops NEW ones appearing. This clears the ones already cached:
-- nulling the syncToken forces one full windowed resync per calendar, and
-- `full_sync` REPLACES the calendar's rows wholesale, so every phantom goes with
-- it. `window_ends_at` is cleared too so the horizon check cannot short-circuit
-- back onto the incremental path before that resync happens.
--
-- Cost: one extra `events.list` pass per selected calendar on the first launch
-- after upgrading. Nothing user-visible is lost — the cache is derived state and
-- `dismissed_calendar_events` (the user's own hides) lives in a separate table.
UPDATE google_calendar_sync
   SET sync_token = NULL,
       window_ends_at = NULL;
