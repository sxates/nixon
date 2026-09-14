-- specs/0029 WS4.3 (1.2 feedback note 13): per-meeting summary template — the persistence
-- slice of specs/0020. Stores the template id the user chose for this meeting (e.g. from the
-- record screen's picker) so the eventual summary generation uses it. NULL — the value for
-- all existing and newly created rows — means "use the default template", so no backfill is
-- needed and template resolution is unchanged for meetings without an explicit choice.
ALTER TABLE meetings ADD COLUMN template_id TEXT;
