-- specs/0053 W3: the section list Auto derives for a meeting.
--
-- Persisted so a regenerated summary keeps its shape, and so the summary cache
-- can fingerprint a stable template (template_cache_fingerprint hashes the
-- RENDERED template, which for Auto is not known until mid-generation).
CREATE TABLE meeting_summary_outlines (
    meeting_id      TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    outline_json    TEXT NOT NULL,
    has_commitments INTEGER NOT NULL,
    derived_at      TEXT NOT NULL
);
