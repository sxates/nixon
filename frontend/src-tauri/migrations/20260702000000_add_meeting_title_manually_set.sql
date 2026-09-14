-- specs/0024 WS6.1 (1.1 feedback note 11): a flag recording that the user manually edited the
-- meeting title, so summary auto-titling never overwrites a name the user chose. Previously the
-- only "don't rename" guard was "has a calendar_event_id", which both (a) left manual renames of
-- ad-hoc meetings unprotected and (b) conflated calendar-adopted titles with user intent.
-- NOT NULL DEFAULT 0 so existing rows read as "not manually set" (eligible for auto-title).
ALTER TABLE meetings ADD COLUMN title_manually_set INTEGER NOT NULL DEFAULT 0;
