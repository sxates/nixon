-- 2.0 Cluster 1 (specs/0015): type a meeting by how it was created.
-- 'recorded' (default, every existing + future recorded meeting) | 'notes_only' | 'imported'.
-- NOT NULL DEFAULT is safe on ALTER ADD in SQLite — existing rows take the default
-- without a backfill UPDATE.
ALTER TABLE meetings ADD COLUMN origin TEXT NOT NULL DEFAULT 'recorded';
