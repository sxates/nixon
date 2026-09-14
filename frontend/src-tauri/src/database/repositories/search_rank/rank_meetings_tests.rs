//! Smoke tests for `rank_meetings_for_question`'s dynamically-built SQL (both
//! the FTS branch and the metadata fallback) against a migrated in-memory DB —
//! the full fixture matrix lives in `tests/aggregation_engine.rs`.
//!
//! Lives beside `search_rank.rs` (not inline) so the production file stays
//! under the specs/0042 file-size ratchet; `super` is `search_rank`.

use super::{SearchRepository, SqlitePool};
use crate::aggregation::scope::AggregationScope;
use crate::database::repositories::meeting::MeetingsRepository;
use sqlx::sqlite::SqlitePoolOptions;

/// Fresh in-memory SQLite brought up through the app's real migration set
/// (mirrors `action_item.rs` tests). One connection max — each in-memory
/// connection is a separate database.
async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn create_meeting(pool: &SqlitePool, title: &str, created_at: &str) -> String {
    let id = MeetingsRepository::create_meeting(pool, Some(title.into()), None, None, None, None)
        .await
        .expect("create_meeting");
    sqlx::query("UPDATE meetings SET created_at = ? WHERE id = ?")
        .bind(created_at)
        .bind(&id)
        .execute(pool)
        .await
        .expect("set created_at");
    id
}

async fn insert_segment(pool: &SqlitePool, meeting_id: &str, id: &str, text: &str) {
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(meeting_id)
    .bind(text)
    .bind("2026-07-01T10:00:00Z")
    .execute(pool)
    .await
    .expect("insert transcript segment");
}

#[tokio::test]
async fn fts_branch_ranks_matching_meeting_with_hit_metadata() {
    let pool = test_pool().await;
    let m1 = create_meeting(&pool, "Pricing sync", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(&pool, "Retro", "2026-06-25T10:00:00Z").await;
    insert_segment(
        &pool,
        &m1,
        "t1",
        "we decided to simplify the pricing page banner",
    )
    .await;
    insert_segment(&pool, &m2, "t2", "retrospective about test flakiness").await;

    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "what did we decide about the pricing page",
        &AggregationScope::default(),
        10,
    )
    .await
    .unwrap();

    assert_eq!(
        ranking.meetings.len(),
        1,
        "only the pricing meeting matches"
    );
    let hit = &ranking.meetings[0];
    assert_eq!(hit.meeting_id, m1);
    assert_eq!(hit.title, "Pricing sync");
    assert_eq!(hit.best_source.as_deref(), Some("transcript"));
    assert_eq!(hit.best_transcript_id.as_deref(), Some("t1"));
    assert_eq!(ranking.dropped, 0);
}

#[tokio::test]
async fn fts_branch_applies_date_and_person_scope() {
    let pool = test_pool().await;
    let m1 = create_meeting(&pool, "Pricing sync", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(&pool, "Pricing follow-up", "2026-06-25T10:00:00Z").await;
    insert_segment(&pool, &m1, "t1", "pricing page discussion").await;
    insert_segment(&pool, &m2, "t2", "pricing page follow up").await;

    // Date range excluding m1 (bounds are full UTC instants).
    let scope = AggregationScope {
        date_from: Some("2026-06-22T00:00:00Z".into()),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert_eq!(ranking.meetings.len(), 1);
    assert_eq!(ranking.meetings[0].meeting_id, m2);

    // date_to is EXCLUSIVE: the start of the day after the selected end
    // day. m2 (2026-06-25T10:00Z) is in for date_to 06-26T00:00Z…
    let scope = AggregationScope {
        date_to: Some("2026-06-26T00:00:00Z".into()),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert_eq!(ranking.meetings.len(), 2);
    // …and out for date_to 06-25T00:00Z (instant comparison, not date()).
    let scope = AggregationScope {
        date_to: Some("2026-06-25T00:00:00Z".into()),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert_eq!(ranking.meetings.len(), 1);
    assert_eq!(ranking.meetings[0].meeting_id, m1);

    // Person on m1's roster only (person_id has no FK enforcement — the
    // pool never enables PRAGMA foreign_keys).
    sqlx::query(
        "INSERT INTO meeting_participants (meeting_id, person_id, source, created_at) \
         VALUES (?, 'person-x', 'manual', '2026-06-20T10:00:00Z')",
    )
    .bind(&m1)
    .execute(&pool)
    .await
    .unwrap();
    let scope = AggregationScope {
        person_id: Some("person-x".into()),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert_eq!(ranking.meetings.len(), 1);
    assert_eq!(ranking.meetings[0].meeting_id, m1);
}

#[tokio::test]
async fn stopword_only_question_falls_back_to_metadata_newest_first_with_cap() {
    let pool = test_pool().await;
    let m1 = create_meeting(&pool, "Older", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(&pool, "Newer", "2026-06-25T10:00:00Z").await;
    insert_segment(&pool, &m1, "t1", "older meeting content").await;
    insert_segment(&pool, &m2, "t2", "newer meeting content").await;

    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "what did we do",
        &AggregationScope::default(),
        1,
    )
    .await
    .unwrap();

    assert_eq!(ranking.meetings.len(), 1, "capped at max_meetings");
    assert_eq!(ranking.meetings[0].meeting_id, m2, "newest first");
    assert_eq!(ranking.meetings[0].best_source, None);
    assert_eq!(ranking.dropped, 1);
    let _ = m1;
}

#[tokio::test]
async fn metadata_fallback_excludes_content_less_meetings_from_cap_and_total() {
    let pool = test_pool().await;
    // Content via transcript, via notes, and none at all.
    let m_transcript = create_meeting(&pool, "Spoken", "2026-06-20T10:00:00Z").await;
    insert_segment(&pool, &m_transcript, "t1", "recorded discussion").await;
    let m_notes = create_meeting(&pool, "Noted", "2026-06-25T10:00:00Z").await;
    sqlx::query(
        "INSERT INTO meeting_notes (meeting_id, notes_markdown, created_at, updated_at) \
         VALUES (?, '# notes', '2026-06-25T10:00:00Z', '2026-06-25T10:00:00Z')",
    )
    .bind(&m_notes)
    .execute(&pool)
    .await
    .unwrap();
    let m_empty = create_meeting(&pool, "Empty shell", "2026-06-28T10:00:00Z").await;
    // Whitespace-only content counts as no content.
    let m_blank = create_meeting(&pool, "Blank", "2026-06-27T10:00:00Z").await;
    insert_segment(&pool, &m_blank, "t2", "   ").await;

    // Uncapped: only the two content-bearing meetings, exact dropped math.
    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "what did we do",
        &AggregationScope::default(),
        10,
    )
    .await
    .unwrap();
    let ids: Vec<&str> = ranking
        .meetings
        .iter()
        .map(|m| m.meeting_id.as_str())
        .collect();
    assert_eq!(ids, vec![m_notes.as_str(), m_transcript.as_str()]);
    assert_eq!(ranking.dropped, 0);
    let _ = m_empty;

    // Capped at 1: the content-less meetings neither occupy a cap slot nor
    // inflate `dropped` — total counts real matches only.
    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "what did we do",
        &AggregationScope::default(),
        1,
    )
    .await
    .unwrap();
    assert_eq!(ranking.meetings.len(), 1);
    assert_eq!(ranking.meetings[0].meeting_id, m_notes);
    assert_eq!(ranking.dropped, 1, "exactly one real match beyond the cap");
}

#[tokio::test]
async fn fts_branch_reports_exact_dropped_beyond_the_cap() {
    let pool = test_pool().await;
    for i in 0..4 {
        let id = create_meeting(
            &pool,
            &format!("Sync {i}"),
            &format!("2026-06-{:02}T10:00:00Z", i + 1),
        )
        .await;
        insert_segment(&pool, &id, &format!("t{i}"), "pricing page discussion").await;
    }

    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "pricing page",
        &AggregationScope::default(),
        2,
    )
    .await
    .unwrap();
    assert_eq!(ranking.meetings.len(), 2, "capped in SQL");
    assert_eq!(ranking.dropped, 2, "exact pre-cap total minus returned");
}

#[tokio::test]
async fn explicit_meeting_ids_never_return_outside_the_set() {
    let pool = test_pool().await;
    let m1 = create_meeting(&pool, "Pricing sync", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(&pool, "Retro", "2026-06-25T10:00:00Z").await;
    insert_segment(&pool, &m1, "t1", "pricing page discussion").await;

    let scope = AggregationScope {
        meeting_ids: Some(vec![m2.clone()]),
        ..Default::default()
    };
    // The question matches m1 — but m1 is outside the pinned set and must
    // never appear (consent pinning). m2 has no hit → appended by metadata.
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert_eq!(ranking.meetings.len(), 1);
    assert_eq!(ranking.meetings[0].meeting_id, m2);
    assert_eq!(ranking.meetings[0].best_source, None);
    assert_eq!(ranking.dropped, 0);

    // An explicitly empty id list selects nothing.
    let scope = AggregationScope {
        meeting_ids: Some(Vec::new()),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert!(ranking.meetings.is_empty());
}

#[tokio::test]
async fn explicit_meeting_ids_keep_fts_hit_metadata_and_append_unmatched() {
    let pool = test_pool().await;
    let m_hit = create_meeting(&pool, "Pricing sync", "2026-06-20T10:00:00Z").await;
    let m_quiet = create_meeting(&pool, "Retro", "2026-06-25T10:00:00Z").await;
    insert_segment(
        &pool,
        &m_hit,
        "t1",
        "we decided to simplify the pricing page banner",
    )
    .await;
    insert_segment(&pool, &m_quiet, "t2", "retrospective about test flakiness").await;

    let scope = AggregationScope {
        meeting_ids: Some(vec![m_hit.clone(), m_quiet.clone()]),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "what did we decide about the pricing page",
        &scope,
        10,
    )
    .await
    .unwrap();

    assert_eq!(ranking.meetings.len(), 2, "the whole pinned set is kept");
    assert_eq!(ranking.dropped, 0, "pinned scopes never report dropped");
    // The FTS hit ranks first and keeps its best-hit metadata — this is
    // what lets gather append the transcript excerpt (R2).
    assert_eq!(ranking.meetings[0].meeting_id, m_hit);
    assert_eq!(
        ranking.meetings[0].best_source.as_deref(),
        Some("transcript")
    );
    assert_eq!(
        ranking.meetings[0].best_transcript_id.as_deref(),
        Some("t1")
    );
    // The un-hit pinned id is appended after with metadata only.
    assert_eq!(ranking.meetings[1].meeting_id, m_quiet);
    assert_eq!(ranking.meetings[1].best_source, None);

    // max_meetings still applies as a hard upper bound.
    let capped = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 1)
        .await
        .unwrap();
    assert_eq!(capped.meetings.len(), 1);
    assert_eq!(capped.dropped, 0);
}

/// specs/0056 W3: the per-meeting top-K transcript hits travel with the row
/// whatever source won, so gather can attach evidence the summary lacks.
#[tokio::test]
async fn transcript_hit_ids_are_the_top_three_segments_by_rank() {
    let pool = test_pool().await;
    let m = create_meeting(&pool, "Pricing sync", "2026-06-20T10:00:00Z").await;
    // Equal-length segments differ only in term frequency (bm25: higher tf
    // ranks better), plus one long single-hit segment (length-normalised
    // below the short single-hit one) and one non-matching segment.
    insert_segment(&pool, &m, "t0", "nothing relevant here").await;
    insert_segment(&pool, &m, "t1", "pricing pricing banner").await;
    insert_segment(
        &pool,
        &m,
        "t2",
        "pricing plus eleven more filler words one two three four five six seven",
    )
    .await;
    insert_segment(&pool, &m, "t3", "pricing pricing pricing").await;
    insert_segment(&pool, &m, "t4", "pricing banner talk").await;

    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "pricing",
        &AggregationScope::default(),
        10,
    )
    .await
    .unwrap();
    assert_eq!(ranking.meetings.len(), 1);
    let hit = &ranking.meetings[0];
    assert_eq!(hit.best_source.as_deref(), Some("transcript"));
    assert_eq!(hit.best_transcript_id.as_deref(), Some("t3"));
    assert_eq!(
        hit.transcript_hit_ids,
        vec!["t3".to_string(), "t1".to_string(), "t4".to_string()],
        "top three by bm25, best first; t2 (4th) and t0 (no hit) are excluded"
    );
}

#[tokio::test]
async fn transcript_hit_ids_are_empty_without_fts_evidence() {
    let pool = test_pool().await;
    let m_hit = create_meeting(&pool, "Pricing sync", "2026-06-20T10:00:00Z").await;
    let m_quiet = create_meeting(&pool, "Retro", "2026-06-25T10:00:00Z").await;
    insert_segment(&pool, &m_hit, "t1", "pricing page discussion").await;
    insert_segment(&pool, &m_quiet, "t2", "retrospective about test flakiness").await;

    // Metadata fallback (stopword-only question): no FTS pass ran.
    let ranking = SearchRepository::rank_meetings_for_question(
        &pool,
        "what did we do",
        &AggregationScope::default(),
        10,
    )
    .await
    .unwrap();
    assert_eq!(ranking.meetings.len(), 2);
    assert!(ranking
        .meetings
        .iter()
        .all(|m| m.transcript_hit_ids.is_empty()));

    // Pinned set: the hit carries its ids, the appended no-hit id carries none.
    let scope = AggregationScope {
        meeting_ids: Some(vec![m_hit.clone(), m_quiet.clone()]),
        ..Default::default()
    };
    let ranking = SearchRepository::rank_meetings_for_question(&pool, "pricing page", &scope, 10)
        .await
        .unwrap();
    assert_eq!(ranking.meetings.len(), 2);
    assert_eq!(ranking.meetings[0].meeting_id, m_hit);
    assert_eq!(
        ranking.meetings[0].transcript_hit_ids,
        vec!["t1".to_string()]
    );
    assert_eq!(ranking.meetings[1].meeting_id, m_quiet);
    assert!(ranking.meetings[1].transcript_hit_ids.is_empty());
}
