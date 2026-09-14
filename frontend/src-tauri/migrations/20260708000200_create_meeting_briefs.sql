-- specs/0036 (pre-call prep): cached pre-meeting BRIEF, one row per target meeting (the
-- upcoming/scheduled occurrence the brief is FOR). Generated in the background from the
-- recurring series' recent prior occurrences via the 0035 aggregation engine, so opening a
-- meeting's prep is instant (no LLM wait).
--
-- Kept OUT of `summary_processes`: that table is one-row-per-meeting keyed to the
-- transcript→summary lifecycle (PENDING/completed/failed + the regenerate backup); a
-- pre-meeting brief is a different artifact (generated before the meeting, from history) and
-- would collide with the real summary row. Modeled instead on the `action_item_extractions`
-- ledger — a fingerprint-invalidated, pre-generated cache.
CREATE TABLE IF NOT EXISTS meeting_briefs (
    meeting_id         TEXT PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    -- 'pending'  — generation in flight (or queued)
    -- 'ready'    — brief_markdown + sources_json are populated
    -- 'failed'   — generation failed (retried on the next pass / manual regenerate)
    -- 'none'     — no prior occurrences with content → there is no brief to show
    status             TEXT NOT NULL DEFAULT 'pending',
    brief_markdown     TEXT,                        -- synthesized brief, [M#] markers post-processed
    sources_json       TEXT,                        -- JSON array of SourceMeeting (the prior occurrences)
    -- Fingerprint of the generation INPUT: the prior occurrence ids + each one's summary
    -- version stamp. When it still matches, the background pass skips regeneration; when a
    -- prior summary changes or a newer occurrence completes, it flips and the brief is rebuilt.
    source_fingerprint TEXT,
    model_provider     TEXT,
    model_name         TEXT,
    generated_at       TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);
