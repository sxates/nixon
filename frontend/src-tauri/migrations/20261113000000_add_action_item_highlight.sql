-- specs/0088 follow-up: the owner highlights to-do items (OneNote-style ⌘4 / ⌘5).
-- NULL = no highlight. Read only by the To-do list view.
ALTER TABLE action_items ADD COLUMN highlight TEXT CHECK (highlight IN ('yellow', 'green'));
