-- specs/0089 follow-up: a topic's to-do card keeps a completed item, ticked off, until the
-- owner dismisses it there. NULL = never dismissed. An item shows again when it is
-- completed after its last dismissal (completed_at > topic_cleared_at), so a reopened and
-- re-completed item is not hidden by an old dismissal. Read only by the topic card query.
ALTER TABLE action_items ADD COLUMN topic_cleared_at TEXT;
