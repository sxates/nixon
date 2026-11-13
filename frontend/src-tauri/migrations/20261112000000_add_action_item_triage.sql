-- specs/0088: to-do list + suggestion inbox. Additive and forward-only.
--   triage: 'suggested' (inbox) | 'listed' (on the owner's to-do list) | 'record' (someone else's)
ALTER TABLE action_items ADD COLUMN triage TEXT NOT NULL DEFAULT 'suggested';
ALTER TABLE action_items ADD COLUMN adopted_at TEXT;      -- when it entered the list
ALTER TABLE action_items ADD COLUMN notes_markdown TEXT;  -- owner's notes (detail page)

-- Every meeting an item was mentioned in. action_items.meeting_id stays the first source.
CREATE TABLE IF NOT EXISTS action_item_sources (
    action_item_id TEXT NOT NULL,      -- action_items.id
    meeting_id     TEXT NOT NULL,      -- meetings.id
    context        TEXT,               -- one line: what was said about it in this meeting
    phrasing       TEXT,               -- the description as extracted from this meeting
    added_at       TEXT NOT NULL,
    PRIMARY KEY (action_item_id, meeting_id)
);
CREATE INDEX IF NOT EXISTS idx_ai_sources_meeting ON action_item_sources(meeting_id);

-- @person mentions in a title. A mention is not the assignee.
CREATE TABLE IF NOT EXISTS action_item_people (
    action_item_id TEXT NOT NULL,      -- action_items.id
    person_id      TEXT NOT NULL,      -- people.id
    PRIMARY KEY (action_item_id, person_id)
);
CREATE INDEX IF NOT EXISTS idx_ai_people_person ON action_item_people(person_id);

-- Backfill: one source row per existing item that has a meeting.
INSERT OR IGNORE INTO action_item_sources (action_item_id, meeting_id, context, phrasing, added_at)
  SELECT id, meeting_id, NULL, description, created_at FROM action_items WHERE meeting_id IS NOT NULL;
UPDATE action_items SET triage = CASE
  WHEN source = 'manual' THEN 'listed'
  WHEN assignee_is_self = 1 AND (status = 'completed' OR user_edited = 1
       OR due_date IS NOT NULL OR sort_order IS NOT NULL) THEN 'listed'
  WHEN assignee_is_self = 1 OR (assignee_person_id IS NULL AND assignee_raw IS NULL) THEN 'suggested'
  ELSE 'record' END;
UPDATE action_items SET adopted_at = updated_at WHERE triage = 'listed';
-- Number the list densely (0..n) so the first item added afterwards (MAX + 1) lands last:
-- rows that already had an order keep it, the rest follow by age.
UPDATE action_items SET sort_order = (
  SELECT rn FROM (
    SELECT id, ROW_NUMBER() OVER (ORDER BY sort_order IS NULL, sort_order, created_at, id) - 1 AS rn
    FROM action_items WHERE triage = 'listed') n
  WHERE n.id = action_items.id)
WHERE triage = 'listed';
