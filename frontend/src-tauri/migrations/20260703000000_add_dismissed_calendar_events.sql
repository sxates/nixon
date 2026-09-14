-- specs/0026 (1.1 feedback note 8): let the user hide non-meeting calendar items (a colleague's
-- doctor's appointment, personal blocks) from the home agenda. Keyed by the EventKit event id as
-- the agenda surfaces it (DayAgendaItem.id for an unrecorded calendar row; a synthetic evt-<hash>
-- when EventKit supplies no id). Persisted so a dismissal survives restarts. Recurring events can
-- share one id across occurrences — dismissing hides all occurrences of that event, which matches
-- "ignore this thing on my calendar".
CREATE TABLE IF NOT EXISTS dismissed_calendar_events (
    event_id     TEXT PRIMARY KEY,
    dismissed_at TEXT NOT NULL
);
