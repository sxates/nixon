-- specs/0089: Topics. Additive, forward-only.
CREATE TABLE IF NOT EXISTS topics (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    status           TEXT NOT NULL DEFAULT 'open',
    notes_markdown   TEXT,
    notes_json       TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    closed_at        TEXT
);

CREATE TABLE IF NOT EXISTS topic_links (
    topic_id   TEXT NOT NULL,
    kind       TEXT NOT NULL,
    ref        TEXT NOT NULL,
    title      TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (topic_id, kind, ref)
);

CREATE TABLE IF NOT EXISTS topic_entries (
    id                TEXT PRIMARY KEY,
    topic_id          TEXT NOT NULL,
    meeting_id        TEXT,
    kind              TEXT NOT NULL,
    occurred_at       TEXT NOT NULL,
    ai_body_json      TEXT,
    body_json         TEXT NOT NULL,
    edited_at         TEXT,
    source_deleted_at TEXT,
    created_at        TEXT NOT NULL,
    UNIQUE (topic_id, meeting_id)
);

CREATE TABLE IF NOT EXISTS topic_revisions (
    id                TEXT PRIMARY KEY,
    entry_id          TEXT NOT NULL,
    decision_id       TEXT,
    text              TEXT NOT NULL,
    source_meeting_id TEXT,
    t_start           REAL,
    source_deleted_at TEXT,
    created_at        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS topic_suggestions (
    id                 TEXT PRIMARY KEY,
    topic_id           TEXT NOT NULL,
    kind               TEXT NOT NULL,
    meeting_id         TEXT NOT NULL,
    t_start            REAL,
    target_entry_id    TEXT,
    target_decision_id TEXT,
    text               TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'pending',
    created_at         TEXT NOT NULL,
    resolved_at        TEXT
);

CREATE TABLE IF NOT EXISTS topic_mention_scans (
    meeting_id  TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

ALTER TABLE action_items ADD COLUMN topic_id TEXT;

CREATE INDEX IF NOT EXISTS idx_topic_entries_topic_time ON topic_entries (topic_id, occurred_at);
CREATE INDEX IF NOT EXISTS idx_topic_suggestions_topic_status ON topic_suggestions (topic_id, status);
CREATE INDEX IF NOT EXISTS idx_topic_suggestions_meeting ON topic_suggestions (meeting_id);
CREATE INDEX IF NOT EXISTS idx_topic_links_kind_ref ON topic_links (kind, ref);
CREATE INDEX IF NOT EXISTS idx_action_items_topic ON action_items (topic_id);
