# 0033 — Full-text search (FTS5) over transcripts, summaries, and notes

- **Status:** Done (implemented 2026-07-02; review-hardened + smoke-verified 2026-07-03)
- **Owner agent(s):** rust-core-engineer (migration, queries, commands, tests) + frontend-engineer (⌘K palette, deep-link)
- **Roadmap phase:** Next #2 (2026-07 roadmap; split from `specs/0021` per the [2026-07-01 roadmap review](../docs/reviews/2026-07-01-roadmap-review.md) §4/§5.1)

## Context / Problem

The search affordance has been reported broken in two of three dogfood rounds (0019 note 11,
0029 WS6.1). As of 1.3 the dashboard "Search meetings" pill (`frontend/src/app/page.tsx:360`)
is a real button — but it opens the ⌘K palette, which only filters meeting **titles**
client-side over `api_get_meetings` (`frontend/src/components/CommandPalette/index.tsx:78-97,
141-166`, cmdk `value`/`keywords` matching). The user cannot answer "which meeting did we
discuss X in" — the core retrieval promise of a meeting assistant — and every week of
dogfooding grows the corpus that makes anything `LIKE`-based less viable.

**Grounded current state:**

- Orphaned backend: `api_search_transcripts` (`frontend/src-tauri/src/api/api.rs:377-399`) →
  `TranscriptsRepository::search_transcripts` (`database/repositories/transcript.rs:314-350`),
  a naive `LIKE '%q%'` with hand-rolled `get_match_context` (`:351-410`, patched in 0028 for
  a non-ASCII char-boundary panic — direct evidence the corpus contains accents/CJK). Exposed
  via `SidebarProvider.searchTranscripts` (`frontend/src/components/Sidebar/SidebarProvider.tsx:198-215`)
  with **zero consumers**. No FTS anywhere in `frontend/src-tauri/migrations/`.
- **Transcripts:** `transcripts(id TEXT PK, meeting_id, transcript, timestamp, …, speaker, channel)`
  (`migrations/20250916100000_initial_schema.sql:10-19`). Writes: `insert_segments`
  (`repositories/transcript.rs:12-52`, shared by `save_transcript` and
  `save_transcripts_for_meeting`). **Rewrites:** "Transcribe now" (specs/0029 deferred
  transcription) calls `replace_meeting_transcripts` (`src/audio/retranscription.rs:545-590`)
  — `DELETE FROM transcripts WHERE meeting_id = ?` then fresh `INSERT`s with **new ids/rowids**.
  Speaker-only updates (`UPDATE transcripts SET speaker = …` in `repositories/speaker.rs:227,302,382`
  and `transcript_speaker_overrides.rs:34,106`) never touch the text. Meeting deletion deletes
  transcripts/summaries/notes **explicitly** inside a transaction (`repositories/meeting.rs:580-602`),
  so we do not depend on FK-cascade trigger semantics.
- **Summaries:** there is no `summaries` table. The final summary is one JSON blob in
  `summary_processes.result` — `{"markdown": …, "english_cache": …}` built by
  `build_summary_result_json` (`src/summary/service.rs:161-175`), written by
  `update_process_completed` (`repositories/summary.rs:121-152`) and by the user-edit path
  `update_meeting_summary` (`repositories/summary.rs:21-58`, called from
  `summary/commands.rs:88`). Regeneration stashes/restores via `result_backup` (`:96-118,154-215`)
  — `result` gets rewritten on completion, failure-restore, and cancellation.
- **Notes:** `meeting_notes(meeting_id TEXT PK, notes_markdown, notes_json, …, enhanced_markdown, enhanced_json)`
  (`migrations/20251223000000_add_meeting_notes.sql`, `20260623000000_add_enhanced_notes.sql`).
  The BlockNote JSON problem is already solved upstream of us: `notes_markdown` is the
  **canonical markdown twin** maintained on every save (`frontend/src/app/_components/NotepadPanel.tsx:18-24,102`)
  and is what the summary pipeline consumes — we index the markdown columns, never parse
  `notes_json`. Writes go through one upsert: `MeetingNotesRepository::upsert_notes`
  (`repositories/meeting_note.rs:11-56`, `INSERT … ON CONFLICT DO UPDATE`).
- **1.3 lifecycle invariant (ROADMAP "Planning invariant"):** record-only mode means a meeting
  row can exist for **days with zero transcripts**; notes-only meetings (specs/0015) may
  *never* have one. Audio-retention sweeps (`src/audio/retention.rs`) delete audio files only —
  transcripts stay, so the index is unaffected by retention.
- **FTS5 availability:** `sqlx = "0.8"` with the `sqlite` feature (`frontend/src-tauri/Cargo.toml:147`)
  bundles SQLite via `libsqlite3-sys 0.30.1` (root `Cargo.lock`), whose bundled build compiles
  with `-DSQLITE_ENABLE_FTS5` and `-DSQLITE_ENABLE_JSON1` (verified in its `build.rs:129`).
  FTS5 + `json_extract`/`json_valid` and generated columns are available. We still assert it
  (see Design) so a future sqlx/feature change fails loudly, not silently.

## Goals

- A phrase spoken in a meeting, written in a summary, or typed in notes is findable from ⌘K,
  with a ranked `snippet()` excerpt and a deep-link that opens the meeting (and scrolls to the
  matched transcript segment when possible).
- The index is maintained by **write triggers on the content tables** — correct under
  record-only meetings, late "Transcribe now" rewrites (delete+reinsert), summary
  regeneration/edit, notes upserts, and meeting deletion. Zero-transcript and notes-only
  meetings are first-class.
- Non-ASCII (accented/CJK) content and queries work: `unicode61 remove_diacritics 2` tokenizer.
- One-time backfill migration indexes the existing corpus.
- Startup (and a cargo test) asserts FTS5 is compiled into the bundled SQLite.
- Replace the orphaned `LIKE` search path rather than leaving two search backends.

## Non-goals

- **Ask-AI / cross-meeting question answering** — the other half of `specs/0021`. It stays in
  Later and folds into the aggregation-engine spec (0013 Wave 3a) per the roadmap review §4;
  this spec is search-only, zero egress.
- Semantic/vector search or embeddings — keyword FTS only this round.
- Indexing people names, participants, or calendar fields (decided 2026-07-02: titles/people
  stay out of the index in v1, keeping the existing client-side title matching tier; folding
  them in later is a small follow-up migration).
- A dedicated search results page. The ⌘K palette is the only surface in v1.
- Windows/Linux — macOS only, as with everything else.

## Approach

Three **external-content FTS5 tables** (`content=` the real tables, so text is stored once)
over `transcripts.transcript`, a new *virtual generated column* extracting
`summary_processes.result → $.markdown`, and `meeting_notes.notes_markdown` +
`enhanced_markdown` — each kept in sync by `AFTER INSERT/UPDATE/DELETE` triggers and
backfilled with the standard `INSERT INTO fts(fts) VALUES('rebuild')`. Trigger-based sync is
what makes the 1.3 lifecycle a non-event: "Transcribe now"'s delete+reinsert and the summary
`result` rewrites fire the same triggers as any other write, so there is no
meeting-end/app-level hook to forget.

The generated-column trick (rather than a self-maintained contentful FTS table for summaries)
keeps all three sources on the identical external-content + triggers pattern and avoids
duplicating summary text; `json_valid()` guards legacy/odd `result` blobs. Alternative
considered: parsing the summary JSON in Rust and writing a separate `search_documents` table —
rejected as a second write path that every summary code path would have to remember to call
(the exact failure mode triggers exist to prevent).

Querying is one new repository (`SearchRepository`) that sanitizes the raw query into a safe
FTS5 MATCH expression (quoted tokens, prefix-star on the last token), UNIONs the three FTS
tables, keeps the best hit per (meeting, source) by bm25, and returns `snippet()` excerpts
delimited by sentinel characters the frontend converts to `<mark>`. The ⌘K palette gains a
debounced backend call and a second result tier: "Meetings" (title matches, existing
client-side behavior, unchanged when the query is empty) then "In transcripts, summaries &
notes" (snippet hits). The old `api_search_transcripts`/`LIKE` path and its dead
`SidebarProvider` plumbing are removed.

## Design

### Data model

One forward-only migration, `frontend/src-tauri/migrations/20260706000000_add_fts5_search.sql`
(follow the header-comment conventions of `20260624000000_add_speakers_table.sql`). Contents:

1. **Summary text extraction column** (virtual, so no table rewrite):

   ```sql
   ALTER TABLE summary_processes ADD COLUMN summary_text TEXT
     GENERATED ALWAYS AS (
       CASE WHEN result IS NOT NULL AND json_valid(result)
            THEN json_extract(result, '$.markdown') END
     ) VIRTUAL;
   ```

   Indexes only the display summary, not `english_cache` (a translation-cache duplicate,
   `summary/service.rs:167-175`) and not `result_backup`.

2. **Three FTS tables**, all `tokenize = "unicode61 remove_diacritics 2"`:

   ```sql
   CREATE VIRTUAL TABLE transcripts_fts USING fts5(
     transcript, meeting_id UNINDEXED,
     content='transcripts', content_rowid='rowid',
     tokenize="unicode61 remove_diacritics 2");
   -- summaries_fts: summary_text + meeting_id UNINDEXED, content='summary_processes'
   -- meeting_notes_fts: notes_markdown + enhanced_markdown + meeting_id UNINDEXED, content='meeting_notes'
   ```

   `meeting_id UNINDEXED` rides along for joins/grouping without a content-table join per hit.

3. **Triggers** — the standard external-content trio per table:
   - `transcripts`: `AFTER INSERT`, `AFTER DELETE`, `AFTER UPDATE OF transcript` (scoped so
     the frequent speaker-relabel `UPDATE`s from diarization/renames don't churn the index).
   - `summary_processes`: `AFTER INSERT`, `AFTER DELETE`, `AFTER UPDATE OF result`
     (covers `update_process_completed`, failure/cancel restores, and the user-edit path).
     `new.summary_text`/`old.summary_text` are computed from the generated column.
   - `meeting_notes`: `AFTER INSERT`, `AFTER DELETE`, `AFTER UPDATE OF notes_markdown, enhanced_markdown`
     (covers both arms of `upsert_notes` and enhancement writes).

4. **Backfill:** `INSERT INTO transcripts_fts(transcripts_fts) VALUES('rebuild');` (ditto the
   other two). Rebuild reads whatever exists — meetings with zero transcripts or no summary
   row contribute nothing and cost nothing, satisfying the record-only/notes-only invariant.

**Correctness notes to encode as migration comments:** external-content FTS keys on the
content table's implicit `rowid` (these tables have TEXT PKs, so rowid is implicit). Nothing
in the codebase runs `VACUUM` today (verified); if a future change adds one, it must be
followed by the `'rebuild'` command, since VACUUM may renumber implicit rowids. Meeting
deletion is safe because `delete_meeting_data` (`repositories/meeting.rs:580-602`) issues
explicit child `DELETE`s, which fire the triggers.

**FTS5 assertion:** in `database/setup.rs::initialize_database_on_startup` (before
`sqlx::migrate!` runs in `manager.rs:54`), probe with
`CREATE VIRTUAL TABLE temp.__fts5_probe USING fts5(x)` + `DROP` and return a loud, actionable
error if it fails (a `pragma compile_options` check is flakier across bundled builds). The
migration itself also fails loudly if FTS5 is missing — the probe just makes the error message
say *why*. Mirror the probe as a cargo test.

### Tauri IPC

- **New:** `api_search_meetings(query: String, limit: Option<u32>) -> Vec<MeetingSearchHit>`
  in `frontend/src-tauri/src/api/api.rs`, registered in `src/lib.rs` (near the old
  registration at `:827`). Backed by a new `database/repositories/search.rs`
  (`SearchRepository`), added to `repositories/mod.rs`.

  ```rust
  pub struct MeetingSearchHit {
      pub meeting_id: String,
      pub title: String,          // joined from meetings
      pub created_at: String,
      pub source: String,         // "transcript" | "summary" | "notes"
      pub snippet: String,        // sentinel-delimited, see below
      pub transcript_id: Option<String>, // best-matching segment, transcript hits only
      pub rank: f64,              // bm25 (lower = better)
  }
  ```

  Query pipeline in `SearchRepository::search`:
  1. **Sanitize** the raw string into an FTS5 MATCH expression: split on whitespace, escape
     internal `"` by doubling, wrap every token in `"…"`, suffix the final token with `*`
     for prefix-typing. Never pass user input to MATCH raw (bare `AND`/`"`/`(` are syntax
     errors — this is the FTS analog of the 0028 non-ASCII panic).
  2. Per-source SELECTs using `snippet(fts, 0, char(1), char(2), '…', 12)` and `bm25()`,
     `GROUP BY meeting_id` keeping `MIN(rank)` per source so one meeting can't flood the list.
  3. UNION, `ORDER BY rank`, `LIMIT` (default 20), join `meetings` for
     title/created_at. Snippets use `\u{1}`/`\u{2}` sentinels so the frontend never renders
     backend HTML.
- **Removed:** `api_search_transcripts` (`api/api.rs:377-399`) and the `LIKE`-based
  `TranscriptsRepository::search_transcripts` + `get_match_context`/char-boundary helpers
  (`repositories/transcript.rs:314-410`) **and their unit tests** (`:413-440` — their
  non-ASCII cases are re-homed as FTS fixtures). Keep `TranscriptSearchResult` only if the new
  hit struct doesn't subsume it (it does — delete it from `api/api.rs:58-64`).
- No new events; search is request/response.

### UI

- **`frontend/src/components/CommandPalette/index.tsx`** (frontend-engineer):
  - Keep the existing Actions group and the client-side title tier exactly as-is for the
    empty query (acceptance: empty-index/empty-query behavior unchanged).
  - When the query is non-empty (≥ 2 chars), debounce (200 ms) an
    `invoke('api_search_meetings', { query })` and render a third group, "In transcripts,
    summaries & notes": icon per `source`, meeting title, date (reuse `formatMeetingDate`),
    and the snippet with sentinel spans rendered as `<mark>`. Switch cmdk to
    `shouldFilter={false}` while a backend query is active and filter the title tier manually
    (cmdk cannot mix client-filtered and server-provided items otherwise).
  - Selecting a hit → `router.push('/meeting-details?id=<meetingId>' + (transcriptId ? '&segment=<id>' : ''))`.
    Errors fall back to the title-only tier with a quiet console warning (palette never breaks).
- **`frontend/src/app/meeting-details/`**: honor a `segment` query param by scrolling the
  transcript view to the matching segment — `VirtualizedTranscriptView.tsx` already assigns
  `id={'segment-' + id}` (`:159`) and has programmatic scrolling (`:262-263`); scroll by index
  lookup on the loaded segments, silently no-op if the id no longer exists (transcript ids are
  regenerated by "Transcribe now", so segment links are best-effort by design).
- **Dashboard pill** (`app/page.tsx:360`): unchanged in v1 — it opens the palette
  (decided 2026-07-02).
- **Cleanup:** remove the dead `searchTranscripts` plumbing from
  `components/Sidebar/SidebarProvider.tsx` (`:50, :198-215, :324, :343`) and its
  `TranscriptSearchResult` type import.

## Tasks

1. [ ] **FTS5 availability guard** — startup probe in
   `frontend/src-tauri/src/database/setup.rs` + a cargo test asserting the probe passes on
   the bundled SQLite (and documenting the `libsqlite3-sys` bundled-build dependency).
   *(rust-core-engineer)*
2. [ ] **Migration** `migrations/20260706000000_add_fts5_search.sql`: `summary_text` generated
   column, three external-content FTS tables (`unicode61 remove_diacritics 2`), nine triggers,
   three `'rebuild'` backfills, correctness comments (rowid/VACUUM, explicit-delete reliance).
   *(rust-core-engineer)*
3. [ ] **`SearchRepository`** (`database/repositories/search.rs` + `mod.rs`): query sanitizer
   (unit-tested against `"` / `AND` / `(` / empty / non-ASCII inputs), UNION query, bm25
   grouping, sentinel snippets. *(rust-core-engineer)*
4. [ ] **IPC swap**: add `api_search_meetings` + `MeetingSearchHit` in `api/api.rs`, register
   in `lib.rs`; delete `api_search_transcripts`, `search_transcripts`, `get_match_context`
   and helpers (`repositories/transcript.rs:314-440`), `TranscriptSearchResult`, and the
   `SidebarProvider.tsx` plumbing. *(rust-core-engineer; SidebarProvider edit with
   frontend-engineer)*
5. [ ] **Integration tests** — new `frontend/src-tauri/tests/fts_search.rs` following the
   `db_lifecycle.rs` pattern: (a) insert transcript → hit + snippet; (b) simulate "Transcribe
   now" via `replace_meeting_transcripts` → old phrase gone, new phrase found;
   (c) `delete_meeting_data` → zero hits; (d) notes-only meeting (no transcripts) found via
   `notes_markdown`; (e) summary completed then user-edited → latest text found; (f) accented
   query matches unaccented text and vice versa (`remove_diacritics 2`) + a CJK phrase;
   (g) meeting with zero transcripts doesn't error or appear. *(rust-core-engineer)*
6. [ ] **⌘K palette tier** in `components/CommandPalette/index.tsx`: debounced backend call,
   `shouldFilter` handling, snippet `<mark>` rendering from sentinels, deep-link navigation,
   graceful error fallback. Vitest coverage for sentinel→mark rendering and the empty-query
   path (existing behavior unchanged). *(frontend-engineer)*
7. [ ] **Deep-link scroll**: `segment` param handling in `app/meeting-details/` +
   `VirtualizedTranscriptView` scroll-to-index; no-op on stale ids. *(frontend-engineer)*
8. [ ] **Docs & gate**: CHANGELOG entry, `specs/INDEX.md` status flip, run `/check`
   (full Definition of Done) and the manual smoke below. *(either owner)*

## Acceptance criteria

Tied to the Definition of Done in `/CLAUDE.md` (items 1–4: `cargo check/clippy/test` clean in
`frontend/src-tauri`, `pnpm lint`/`pnpm test` clean in `frontend`, app launches, record →
transcript → summary smoke intact):

- A phrase spoken in a recorded meeting, a phrase only in its summary, and a phrase only in a
  notes-only meeting's notes are each findable via ⌘K, showing a highlighted snippet, and
  selecting the hit opens the right meeting (transcript hits scroll to the segment).
- An accented query (e.g. `resume` ↔ `résumé`) and a CJK phrase round-trip through search
  (fixture-verified).
- After "Transcribe now" rewrites a meeting's transcript, searching finds the **new** text and
  not the old (fixture b); after deleting a meeting, none of its content is findable (fixture c).
- A meeting with zero transcripts (record-only, not yet transcribed) neither errors nor
  pollutes results; the palette's empty-query behavior (Actions + recent meetings by title) is
  byte-for-byte unchanged.
- Startup fails with an actionable message if FTS5 is absent; the probe test passes in CI.
- Queries containing FTS5 metacharacters (`"`, `(`, `AND`, trailing `*`) return results or
  empty — never an error surfaced to the palette.
- `api_search_transcripts` and the `LIKE` path no longer exist in the codebase.

## Risks / open questions

- **bm25 scores aren't comparable across the three FTS tables** — a UNION `ORDER BY rank`
  mixes scales. Accepted for v1 (per-source `MIN` grouping + a small limit keeps it sane);
  revisit with per-source weighting if dogfooding shows summaries drowning out transcripts.
- **Implicit-rowid coupling**: external-content tables key on rowids of TEXT-PK tables. Safe
  today (no VACUUM anywhere; deletes are explicit); guarded by migration comments and fixture
  (c). A future "compact database" feature must rebuild the index.
- **Trigger overhead on live transcription**: one FTS insert per segment insert. Segments are
  short and writes already per-segment (`insert_segments`); if profiling ever shows pressure,
  FTS5's `automerge`/`'optimize'` knobs exist. Not expected to matter at dogfood scale.
- **Generated columns in triggers** require SQLite ≥ 3.32; bundled libsqlite3-sys 0.30.1
  ships far newer. The startup probe + fixture (e) would catch a regression.
- **Phrase queries are not supported in v1 (review finding, accepted):** the sanitizer quotes
  every whitespace token individually, so a user-typed `"exact phrase"` is searched as AND'd
  terms — matches can be far apart, ranked as if adjacent, with no error. v2: parse balanced
  user quotes into single FTS5 phrase terms (the sanitizer unit tests lock the v1 behavior
  and will need updating deliberately).
- **Single-pass snippet materialization (review finding, benchmarked, accepted):** `SEARCH_SQL`
  computes `snippet()` for every match before GROUP BY/LIMIT. Measured at 100k segments:
  344 ms worst case (stop-word prefix), ~30 ms typical; a two-pass rewrite (rank-only CTEs →
  snippet only the ≤20 winners via `MATCH ? AND rowid IN (…)`) saves ~30% only in the
  worst case. Revisit only if dogfooding shows keystroke latency.
- **Decided 2026-07-02 (owner accepted defaults):** meeting titles/people names stay out of
  the FTS index in v1 (client-side cmdk tier unchanged; server-side titles are a small
  follow-up migration if wanted); debounce 200 ms, result cap 20 hits
  best-per-meeting-per-source; the dashboard pill remains a palette-opener (promoting it to a
  real inline input is out of scope).

## Verification

Automated (all green via `/check`):

```bash
cd frontend/src-tauri && source ~/.cargo/env && \
  cargo check && cargo clippy && \
  cargo test --features metal --test fts_search --test db_lifecycle
cd frontend && pnpm lint && pnpm test
```

- `fts_search.rs` fixtures (a)–(g) from task 5, including the rewrite-then-search and
  delete-then-search invariants and the FTS5 probe test.
- Sanitizer unit tests in `repositories/search.rs`.
- Vitest: palette snippet rendering + unchanged empty-query snapshot.

Manual smoke (dev build, `./dev-vinyl.sh`, existing dogfood DB — exercises the backfill):

1. Launch → confirm no startup error; migration applies once; search immediately finds a
   phrase from a **pre-existing** meeting (proves `'rebuild'` backfill).
2. ⌘K → type a phrase you remember saying in a real meeting → snippet appears with the phrase
   highlighted → Enter opens the meeting and scrolls to the segment.
3. Search a phrase that exists only in a meeting's notes (notes-only meeting) and only in a
   summary → both found, labeled by source.
4. On a record-only meeting: search before transcription (no hit, no error) → "Transcribe
   now" → the spoken phrase becomes findable without restarting the app.
5. Empty query: palette shows Actions + recent meetings exactly as before this change.
6. Record → live transcript → summary end-to-end still works (DoD item 4).
