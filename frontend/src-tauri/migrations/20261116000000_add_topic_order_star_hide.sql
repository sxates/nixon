-- specs/0090: manual topic order, stars, and "Keep off the To-do list". Additive,
-- forward-only; 0089's topic migrations are released and frozen.
ALTER TABLE topics ADD COLUMN sort_order INTEGER;
ALTER TABLE topics ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;
ALTER TABLE topics ADD COLUMN hide_from_todo INTEGER NOT NULL DEFAULT 0;

-- Backfill order: today's visible order (open first, then name).
UPDATE topics SET sort_order = (SELECT rn FROM (
  SELECT id, ROW_NUMBER() OVER (ORDER BY CASE status WHEN 'open' THEN 0 ELSE 1 END,
                                         name COLLATE NOCASE, id) - 1 AS rn FROM topics) r
  WHERE r.id = topics.id);

-- Owner decision: star what the sidebar listed before (its 5 most recently active open
-- topics, by MAX(topic_entries.occurred_at), topics with no entries last), so the
-- starred-only sidebar isn't empty after the upgrade.
UPDATE topics SET starred = 1 WHERE id IN (
  SELECT t.id FROM topics t WHERE t.status = 'open'
  ORDER BY (SELECT MAX(e.occurred_at) FROM topic_entries e WHERE e.topic_id = t.id) IS NULL,
           (SELECT MAX(e.occurred_at) FROM topic_entries e WHERE e.topic_id = t.id) DESC
  LIMIT 5);
