-- specs/0069 W3 — a meeting you add inside Nixon is a `scheduled` prep row with a
-- Nixon-minted event id. Two things the calendar supplied for every other scheduled row
-- have nowhere to live: the occurrence END (the timeline needs it to size a block) and a
-- join link. Both are NULL for every existing row and are written only by manual entries.
ALTER TABLE meetings ADD COLUMN scheduled_end_at TEXT;
ALTER TABLE meetings ADD COLUMN join_url TEXT;
