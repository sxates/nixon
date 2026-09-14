-- specs/0041 WS4: manual series association for pre-call prep.
--
-- A row pins one meeting into a recurring series regardless of its calendar link or
-- title, so prior-occurrence matching (find_prior_series_occurrences) can find ad-hoc
-- recordings, renamed recurring meetings, and one-off events the owner marks as related.
--
-- `series_key` is either an existing calendar series key (EventKit
-- calendarItemExternalIdentifier / Google iCalUID, i.e. meetings.calendar_series_key)
-- or a minted synthetic `manual:{uuid}` key when neither side has one. We NEVER write
-- meetings.calendar_series_key itself — that column stays calendar-owned; manual
-- association lives entirely in this table.
CREATE TABLE IF NOT EXISTS meeting_series_links (
    meeting_id TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    series_key TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_meeting_series_links_series_key
    ON meeting_series_links(series_key);
