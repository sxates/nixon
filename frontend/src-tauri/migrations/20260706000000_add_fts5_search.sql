-- Migration: full-text search over transcripts, summaries, and notes (specs/0033)
--
-- Three EXTERNAL-CONTENT FTS5 tables (content= the real tables, so text is stored
-- once) kept in sync by AFTER INSERT / AFTER DELETE / AFTER UPDATE OF triggers on
-- the content tables, then backfilled once with the standard
-- INSERT INTO <fts>(<fts>) VALUES('rebuild').
--
-- Trigger-based sync makes the 1.3 lifecycle a non-event: "Transcribe now"'s
-- delete+reinsert (audio/retranscription.rs::replace_meeting_transcripts), summary
-- regeneration/edit/restore rewrites of summary_processes.result, and the
-- meeting_notes upsert all fire the same triggers as any other write — there is no
-- app-level hook to forget.
--
-- CORRECTNESS NOTES (do not break these invariants):
--
-- * rowid coupling: external-content FTS keys on the content table's rowid. All
--   three content tables have TEXT primary keys, so their rowid is IMPLICIT.
--   Nothing in the codebase runs VACUUM today (verified 2026-07); VACUUM may
--   renumber implicit rowids, so any future change that adds a VACUUM (e.g. a
--   "compact database" feature) MUST follow it with
--   INSERT INTO <fts>(<fts>) VALUES('rebuild') for each of the three FTS tables.
--
-- * explicit-delete reliance: meeting deletion is safe because
--   repositories/meeting.rs::delete_meeting_with_transaction issues EXPLICIT child
--   DELETEs (transcripts, summary_processes, meeting_notes) inside a transaction,
--   which fire the AFTER DELETE triggers below. Do not switch meeting deletion to
--   rely solely on FK ON DELETE CASCADE: cascade deletions only fire child-table
--   triggers when PRAGMA recursive_triggers is enabled, which the app does not set.
--
-- * UPDATE triggers are scoped (UPDATE OF <text columns>) so the frequent
--   speaker-relabel UPDATEs from diarization/renames (repositories/speaker.rs,
--   transcript_speaker_overrides.rs) never churn the index.
--
-- Forward-only (CLAUDE.md). Requires FTS5 + JSON1 + generated columns — all
-- compiled into the bundled SQLite (libsqlite3-sys `bundled`); asserted at startup
-- by database/setup.rs::assert_fts5_available.

-- ---------------------------------------------------------------------------
-- 1. Summary text extraction: a VIRTUAL generated column (no table rewrite)
--    pulling the display markdown out of the summary JSON blob. Indexes only
--    $.markdown — not english_cache (a translation-cache duplicate) and not
--    result_backup. json_valid() guards legacy/odd result blobs.
-- ---------------------------------------------------------------------------
ALTER TABLE summary_processes ADD COLUMN summary_text TEXT
  GENERATED ALWAYS AS (
    CASE WHEN result IS NOT NULL AND json_valid(result)
         THEN json_extract(result, '$.markdown') END
  ) VIRTUAL;

-- ---------------------------------------------------------------------------
-- 2. FTS tables. meeting_id UNINDEXED rides along so grouping/joins don't need
--    a content-table join per hit. unicode61 remove_diacritics 2 folds accents
--    in both the indexed text and the query (résumé <-> resume).
-- ---------------------------------------------------------------------------
CREATE VIRTUAL TABLE IF NOT EXISTS transcripts_fts USING fts5(
  transcript,
  meeting_id UNINDEXED,
  content='transcripts',
  content_rowid='rowid',
  tokenize="unicode61 remove_diacritics 2"
);

CREATE VIRTUAL TABLE IF NOT EXISTS summaries_fts USING fts5(
  summary_text,
  meeting_id UNINDEXED,
  content='summary_processes',
  content_rowid='rowid',
  tokenize="unicode61 remove_diacritics 2"
);

CREATE VIRTUAL TABLE IF NOT EXISTS meeting_notes_fts USING fts5(
  notes_markdown,
  enhanced_markdown,
  meeting_id UNINDEXED,
  content='meeting_notes',
  content_rowid='rowid',
  tokenize="unicode61 remove_diacritics 2"
);

-- ---------------------------------------------------------------------------
-- 3. Triggers — the standard external-content trio per table.
-- ---------------------------------------------------------------------------

-- transcripts: covers insert_segments (live save), import, and "Transcribe now"'s
-- delete+reinsert. UPDATE scoped to `transcript` so speaker relabels are free.
CREATE TRIGGER IF NOT EXISTS transcripts_fts_ai AFTER INSERT ON transcripts BEGIN
  INSERT INTO transcripts_fts(rowid, transcript, meeting_id)
  VALUES (new.rowid, new.transcript, new.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS transcripts_fts_ad AFTER DELETE ON transcripts BEGIN
  INSERT INTO transcripts_fts(transcripts_fts, rowid, transcript, meeting_id)
  VALUES ('delete', old.rowid, old.transcript, old.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS transcripts_fts_au AFTER UPDATE OF transcript ON transcripts BEGIN
  INSERT INTO transcripts_fts(transcripts_fts, rowid, transcript, meeting_id)
  VALUES ('delete', old.rowid, old.transcript, old.meeting_id);
  INSERT INTO transcripts_fts(rowid, transcript, meeting_id)
  VALUES (new.rowid, new.transcript, new.meeting_id);
END;

-- summary_processes: UPDATE OF result covers update_process_completed, the
-- failure/cancel restores (result = result_backup), and the user-edit path
-- update_meeting_summary. old./new.summary_text are computed from the generated
-- column (SQLite >= 3.32; bundled build is far newer).
CREATE TRIGGER IF NOT EXISTS summaries_fts_ai AFTER INSERT ON summary_processes BEGIN
  INSERT INTO summaries_fts(rowid, summary_text, meeting_id)
  VALUES (new.rowid, new.summary_text, new.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS summaries_fts_ad AFTER DELETE ON summary_processes BEGIN
  INSERT INTO summaries_fts(summaries_fts, rowid, summary_text, meeting_id)
  VALUES ('delete', old.rowid, old.summary_text, old.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS summaries_fts_au AFTER UPDATE OF result ON summary_processes BEGIN
  INSERT INTO summaries_fts(summaries_fts, rowid, summary_text, meeting_id)
  VALUES ('delete', old.rowid, old.summary_text, old.meeting_id);
  INSERT INTO summaries_fts(rowid, summary_text, meeting_id)
  VALUES (new.rowid, new.summary_text, new.meeting_id);
END;

-- meeting_notes: UPDATE OF covers both arms of upsert_notes (the ON CONFLICT
-- DO UPDATE arm sets notes_markdown) and future enhancement writes.
CREATE TRIGGER IF NOT EXISTS meeting_notes_fts_ai AFTER INSERT ON meeting_notes BEGIN
  INSERT INTO meeting_notes_fts(rowid, notes_markdown, enhanced_markdown, meeting_id)
  VALUES (new.rowid, new.notes_markdown, new.enhanced_markdown, new.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS meeting_notes_fts_ad AFTER DELETE ON meeting_notes BEGIN
  INSERT INTO meeting_notes_fts(meeting_notes_fts, rowid, notes_markdown, enhanced_markdown, meeting_id)
  VALUES ('delete', old.rowid, old.notes_markdown, old.enhanced_markdown, old.meeting_id);
END;

CREATE TRIGGER IF NOT EXISTS meeting_notes_fts_au
AFTER UPDATE OF notes_markdown, enhanced_markdown ON meeting_notes BEGIN
  INSERT INTO meeting_notes_fts(meeting_notes_fts, rowid, notes_markdown, enhanced_markdown, meeting_id)
  VALUES ('delete', old.rowid, old.notes_markdown, old.enhanced_markdown, old.meeting_id);
  INSERT INTO meeting_notes_fts(rowid, notes_markdown, enhanced_markdown, meeting_id)
  VALUES (new.rowid, new.notes_markdown, new.enhanced_markdown, new.meeting_id);
END;

-- ---------------------------------------------------------------------------
-- 4. One-time backfill of the existing corpus. 'rebuild' reads whatever exists —
--    meetings with zero transcripts or no summary/notes row contribute nothing
--    and cost nothing (record-only / notes-only invariant, specs/0033).
-- ---------------------------------------------------------------------------
INSERT INTO transcripts_fts(transcripts_fts) VALUES('rebuild');
INSERT INTO summaries_fts(summaries_fts) VALUES('rebuild');
INSERT INTO meeting_notes_fts(meeting_notes_fts) VALUES('rebuild');
