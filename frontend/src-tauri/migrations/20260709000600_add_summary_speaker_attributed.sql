-- specs/0041 WS2 — summary waits for / refreshes with speakers.
--
-- `speaker_attributed`: set at generation time from the summary path's `any_speaker`
-- computation (summary/service.rs). 0 means the summary was generated from a
-- transcript with no resolved speaker names (diarization hadn't landed yet); the
-- post-diarization trigger (summary/refresh.rs) only auto-regenerates such summaries,
-- and the regenerated run sets 1, so the trigger can never loop.
--
-- `generated_markdown_hash`: fingerprint (service.rs `stable_text_fingerprint`) of the
-- stored `$.markdown` at generation time. The pristine guard compares it against the
-- current `$.markdown` before auto-regenerating — a mismatch means the user edited the
-- summary since generation, and we never clobber user edits. NULL (legacy rows from
-- before this migration) is treated as "can't prove pristine" → no auto-regenerate.
ALTER TABLE summary_processes ADD COLUMN speaker_attributed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE summary_processes ADD COLUMN generated_markdown_hash TEXT;
