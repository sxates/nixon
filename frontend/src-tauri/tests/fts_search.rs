//! Full-text search integration tests (specs/0033, task 5).
//!
//! Exercises the FTS5 index end-to-end over the app's REAL migration path
//! (`DatabaseManager::new` runs `migrations/20260706000000_add_fts5_search.sql`,
//! including the triggers and the 'rebuild' backfill) and the same repository
//! calls the Tauri commands wrap — no audio, no Tauri runtime (the
//! `db_lifecycle.rs` pattern).
//!
//! Fixtures (a)–(g) from the spec:
//!   (a) insert transcript -> hit + sentinel snippet
//!   (b) "Transcribe now" rewrite via replace_meeting_transcripts -> old gone, new found
//!   (c) meeting delete -> zero hits across all sources
//!   (d) notes-only meeting (no transcripts) found via notes_markdown (+ enhanced)
//!   (e) summary completed then user-edited -> latest text found, stale text gone
//!   (f) diacritics fold both ways (remove_diacritics 2) + a CJK phrase
//!   (g) zero-transcript meeting neither errors nor appears
//! Plus: FTS5 metacharacter queries never error. (The FTS5-availability probe
//! itself is unit-tested next to its code in `database/setup.rs`.)
//!
//! Run with:
//!   cargo test --features metal --test fts_search -- --nocapture

mod common;

use app_lib::audio::retranscription::replace_meeting_transcripts;
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::meeting_note::MeetingNotesRepository;
use app_lib::database::repositories::search::SearchRepository;
use app_lib::database::repositories::summary::SummaryProcessesRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::search::MeetingSearchHit;
use app_lib::transcripts::TranscriptSegment;
use common::{fresh_db, segment};
use serde_json::json;
use sqlx::SqlitePool;

/// Sentinels the snippet() excerpts delimit matches with (see SearchRepository).
const MARK_START: char = '\u{1}';
const MARK_END: char = '\u{2}';

async fn search(pool: &SqlitePool, query: &str) -> Vec<MeetingSearchHit> {
    SearchRepository::search(pool, query, None)
        .await
        .unwrap_or_else(|e| panic!("search for '{query}' must not error: {e}"))
}

/// Create a meeting with one recorded transcript session.
async fn meeting_with_transcripts(pool: &SqlitePool, title: &str, texts: &[&str]) -> String {
    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");
    let segments: Vec<TranscriptSegment> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| segment(t, i as f64 * 2.0, i as f64 * 2.0 + 2.0))
        .collect();
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &meeting_id,
        title,
        &segments,
        None,
    )
    .await
    .expect("save_transcripts_for_meeting");
    assert!(attached, "segments attach to the freshly created meeting");
    meeting_id
}

// ---------------------------------------------------------------------------
// (a) insert -> hit + snippet
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transcript_insert_is_immediately_searchable_with_snippet() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = meeting_with_transcripts(
        pool,
        "Budget sync",
        &[
            "we walked through the quarterly budget forecast in detail",
            "unrelated closing remarks",
        ],
    )
    .await;

    let hits = search(pool, "budget").await;
    assert_eq!(hits.len(), 1, "one hit: best segment per (meeting, source)");
    let hit = &hits[0];
    assert_eq!(hit.meeting_id, meeting_id);
    assert_eq!(hit.source, "transcript");
    assert_eq!(hit.title, "Budget sync");
    assert!(
        !hit.created_at.is_empty(),
        "created_at joined from meetings"
    );
    assert!(
        hit.snippet
            .contains(&format!("{MARK_START}budget{MARK_END}")),
        "snippet must delimit the match with sentinels, got: {:?}",
        hit.snippet
    );
    let tid = hit
        .transcript_id
        .as_deref()
        .expect("transcript hits carry the best-matching segment id");
    assert!(tid.starts_with("transcript-"), "real segment id, got {tid}");

    // Prefix-typing: the final token is prefix-starred.
    let hits = search(pool, "quarterly fore").await;
    assert_eq!(hits.len(), 1, "prefix match on the last token");
}

// ---------------------------------------------------------------------------
// (b) "Transcribe now" rewrite: delete+reinsert with new ids
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transcribe_now_rewrite_replaces_index_content() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id =
        meeting_with_transcripts(pool, "Rewritten", &["draft phrase about pineapples"]).await;
    assert_eq!(search(pool, "pineapples").await.len(), 1);

    // replace_meeting_transcripts inserts ids verbatim, so provide fresh ones
    // (same contract as the retranscription pipeline).
    let mut replacement = segment("final phrase about kubernetes", 0.0, 2.0);
    replacement.id = "transcript-rewrite-1".to_string();
    replace_meeting_transcripts(pool, &meeting_id, &[replacement])
        .await
        .expect("replace_meeting_transcripts");

    assert!(
        search(pool, "pineapples").await.is_empty(),
        "old text must leave the index with its deleted rows"
    );
    let hits = search(pool, "kubernetes").await;
    assert_eq!(hits.len(), 1, "rewritten text is findable");
    assert_eq!(hits[0].meeting_id, meeting_id);
    assert_eq!(
        hits[0].transcript_id.as_deref(),
        Some("transcript-rewrite-1")
    );
}

// ---------------------------------------------------------------------------
// (c) meeting delete -> zero hits across all three sources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn meeting_delete_purges_all_sources_from_index() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id =
        meeting_with_transcripts(pool, "Doomed", &["transcript mentions zeppelins"]).await;
    SummaryProcessesRepository::create_or_reset_process(pool, &meeting_id)
        .await
        .expect("create process");
    // Hand-rolled `result` JSON: the real shape is owned by the pipeline's
    // build_summary_result_json (summary/service.rs), whose `$.markdown` layout
    // is shape-locked against the FTS migration by the unit test
    // `summary_result_json_markdown_is_extractable_by_fts_migration` there.
    SummaryProcessesRepository::update_process_completed(
        pool,
        &meeting_id,
        json!({"markdown": "Summary mentions dirigibles", "english_cache": null}),
        1,
        0.5,
        false,
        &app_lib::summary::refresh::generation_fingerprint("Summary mentions dirigibles"),
        &app_lib::summary::refresh::speaker_names_fingerprint(std::iter::empty()),
    )
    .await
    .expect("complete summary");
    assert!(MeetingNotesRepository::upsert_notes(
        pool,
        &meeting_id,
        Some("notes mention blimps"),
        None
    )
    .await
    .expect("upsert notes"));

    // All three sources indexed before the delete.
    assert_eq!(search(pool, "zeppelins").await.len(), 1);
    assert_eq!(search(pool, "dirigibles").await.len(), 1);
    assert_eq!(search(pool, "blimps").await.len(), 1);

    // The explicit children-first delete (fires the AFTER DELETE triggers).
    let deleted = MeetingsRepository::delete_meeting(pool, &meeting_id)
        .await
        .expect("delete_meeting");
    assert!(deleted);

    for q in ["zeppelins", "dirigibles", "blimps"] {
        assert!(
            search(pool, q).await.is_empty(),
            "'{q}' must be gone after meeting delete"
        );
    }
}

// ---------------------------------------------------------------------------
// (d) notes-only meeting (zero transcripts, never recorded)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn notes_only_meeting_is_found_via_notes() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(
        pool,
        Some("Grocery planning".into()),
        None,
        Some("notes_only".into()),
        None,
        None,
    )
    .await
    .expect("create notes-only meeting");
    assert!(MeetingNotesRepository::upsert_notes(
        pool,
        &meeting_id,
        Some("# Plan\nremember to buy espresso beans"),
        Some(r#"[{"type":"paragraph"}]"#),
    )
    .await
    .expect("upsert notes"));

    let hits = search(pool, "espresso").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meeting_id, meeting_id);
    assert_eq!(hits[0].source, "notes");
    assert!(
        hits[0].transcript_id.is_none(),
        "notes hits carry no segment deep-link"
    );

    // Autosave upsert (the ON CONFLICT DO UPDATE arm) re-indexes.
    assert!(MeetingNotesRepository::upsert_notes(
        pool,
        &meeting_id,
        Some("switched to matcha"),
        None
    )
    .await
    .expect("second upsert"));
    assert!(
        search(pool, "espresso").await.is_empty(),
        "stale notes gone"
    );
    assert_eq!(search(pool, "matcha").await.len(), 1);

    // enhanced_markdown is indexed too (UPDATE OF enhanced_markdown trigger);
    // no repository write path exists yet (spec 0003 pivot), so update directly.
    sqlx::query("UPDATE meeting_notes SET enhanced_markdown = ? WHERE meeting_id = ?")
        .bind("enhanced notes mention zanzibar")
        .bind(&meeting_id)
        .execute(pool)
        .await
        .expect("set enhanced_markdown");
    let hits = search(pool, "zanzibar").await;
    assert_eq!(hits.len(), 1, "enhanced-notes matches are findable");
    assert_eq!(hits[0].source, "notes");
    assert!(
        hits[0].snippet.contains(MARK_START),
        "snippet comes from the matched column (snippet column -1), got: {:?}",
        hits[0].snippet
    );
}

// ---------------------------------------------------------------------------
// (e) summary completed then user-edited -> latest text wins
// ---------------------------------------------------------------------------

#[tokio::test]
async fn summary_completion_and_user_edit_track_latest_text() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = meeting_with_transcripts(pool, "Roadmap", &["hello world"]).await;

    // Pipeline completion (update_process_completed rewrites `result`). JSON
    // shape mirrors build_summary_result_json — see the shape-lock unit test
    // in summary/service.rs.
    SummaryProcessesRepository::create_or_reset_process(pool, &meeting_id)
        .await
        .expect("create process");
    SummaryProcessesRepository::update_process_completed(
        pool,
        &meeting_id,
        json!({"markdown": "## Decisions\nadopt the vinyl roadmap", "english_cache": null}),
        1,
        0.5,
        false,
        &app_lib::summary::refresh::generation_fingerprint("## Decisions\nadopt the vinyl roadmap"),
        &app_lib::summary::refresh::speaker_names_fingerprint(std::iter::empty()),
    )
    .await
    .expect("complete summary");

    let hits = search(pool, "roadmap").await;
    // "Roadmap" is the title (NOT indexed, v1 decision) — the hit must be the summary.
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source, "summary");
    assert_eq!(hits[0].meeting_id, meeting_id);
    assert!(hits[0].transcript_id.is_none());

    // User edit path (update_meeting_summary rewrites `result`).
    let edited = SummaryProcessesRepository::update_meeting_summary(
        pool,
        &meeting_id,
        &json!({"markdown": "## Decisions\npostpone the launch"}),
    )
    .await
    .expect("update_meeting_summary");
    assert!(edited);

    assert!(
        search(pool, "adopt").await.is_empty(),
        "pre-edit summary text must be gone"
    );
    let hits = search(pool, "postpone").await;
    assert_eq!(hits.len(), 1, "edited summary text is findable");
    assert_eq!(hits[0].source, "summary");
}

// ---------------------------------------------------------------------------
// (f) diacritics fold both ways + CJK
// ---------------------------------------------------------------------------

#[tokio::test]
async fn diacritics_fold_both_ways_and_cjk_round_trips() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let accented =
        meeting_with_transcripts(pool, "Hiring", &["she updated her résumé yesterday"]).await;
    let plain = meeting_with_transcripts(pool, "Coffee", &["let us meet at the cafe"]).await;
    let cjk = meeting_with_transcripts(pool, "日本語", &["プロジェクト計画を確認しました"]).await;

    // Unaccented query -> accented text (remove_diacritics 2).
    let hits = search(pool, "resume").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meeting_id, accented);

    // Accented query -> unaccented text.
    let hits = search(pool, "café").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meeting_id, plain);

    // CJK phrase round-trips (unicode61 keeps the run as one token; the
    // sanitizer's trailing prefix-star lets a leading phrase match it).
    let hits = search(pool, "プロジェクト計画").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meeting_id, cjk);
}

// ---------------------------------------------------------------------------
// (g) zero-transcript (record-only) meeting: no error, no pollution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn zero_transcript_meeting_neither_errors_nor_appears() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Record-only: row exists (possibly for days), zero transcripts/summary/notes.
    let empty_meeting = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create record-only meeting");
    // And one real meeting so results are non-trivially non-empty.
    let real = meeting_with_transcripts(pool, "Real", &["discussing the migration"]).await;

    let hits = search(pool, "migration").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meeting_id, real);
    assert!(
        hits.iter().all(|h| h.meeting_id != empty_meeting),
        "the record-only meeting must not appear"
    );

    // A miss is an empty vec, not an error.
    assert!(search(pool, "nonexistentphrase").await.is_empty());
}

// ---------------------------------------------------------------------------
// FTS5 metacharacters: results or empty — never an error (acceptance criterion)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metacharacter_queries_never_error() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    meeting_with_transcripts(pool, "Robust", &["quoting AND (grouping) near budget*"]).await;

    for q in [
        "\"",
        "(",
        ")",
        "AND",
        "OR",
        "NOT",
        "NEAR",
        "budget*",
        "*",
        "^",
        "-",
        ":",
        "\"unclosed",
        "a AND b",
        "((((",
        "",
        "   ",
    ] {
        // `search` panics with a useful message if the query errors.
        let _ = search(pool, q).await;
    }

    // Sanity: quoted-operator queries still MATCH as literals.
    assert_eq!(search(pool, "AND").await.len(), 1, "literal 'and' matches");
    assert_eq!(search(pool, "grouping").await.len(), 1);
}
