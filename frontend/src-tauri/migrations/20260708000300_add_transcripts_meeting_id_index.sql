-- specs/0037 (review-2 finding): transcripts(meeting_id) had NO index, so every
-- per-meeting transcript query — get_full_transcript, excerpts, the resume-append
-- dedupe probe — was a full-table scan. One meeting's rows are read constantly;
-- this is the workhorse index the table always needed.
CREATE INDEX IF NOT EXISTS idx_transcripts_meeting_id ON transcripts(meeting_id);
