//! Pre-call prep integration tests (specs/0036).
//!
//! Exercises the prep pipeline over the app's REAL migration set with an injected FAKE LLM —
//! no network, no Tauri runtime, no provider config. The full `generate_brief_for_target`
//! (which resolves a real provider + emits Tauri events) is covered by the manual smoke; here
//! we test its testable core: prior-occurrence detection → `execute_pre_call_prep` over the
//! pinned set → cited brief, plus the carryover query and the brief cache.

mod common;

use app_lib::aggregation::engine::Stage;
use app_lib::aggregation::{execute_pre_call_prep, SourceMeeting};
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::meeting_brief::MeetingBriefsRepository;
use app_lib::database::repositories::summary::SummaryProcessesRepository;
use common::fresh_db;
use serde_json::json;
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

const GENEROUS_BUDGET: usize = 1_000_000; // forces single-pass reduce

type LlmFuture = Pin<Box<dyn Future<Output = Result<String, String>>>>;

/// Fake LLM for the prep pipeline: the reduce call (recognized by the prep reduce prompt's
/// wording) returns the scripted brief; any other call is a map extraction and echoes a stub.
fn fake_llm(
    reduce_answer: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
) -> impl Fn(String, String) -> LlmFuture {
    move |_system: String, user: String| {
        let calls = calls.clone();
        Box::pin(async move {
            calls.lock().unwrap().push(user.clone());
            if user.contains("Write the brief now") {
                Ok(reduce_answer.to_string())
            } else {
                Ok("DECISION: shipped the thing".to_string())
            }
        })
    }
}

/// Insert a recorded occurrence dated `created_at` with a series key + a completed summary.
async fn occurrence(
    pool: &SqlitePool,
    title: &str,
    created_at: &str,
    series_key: &str,
    summary: &str,
) -> String {
    let id = format!("meeting-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_series_key) \
         VALUES (?, ?, ?, ?, 'recorded', ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(created_at)
    .bind(created_at)
    .bind(series_key)
    .execute(pool)
    .await
    .expect("insert occurrence");
    SummaryProcessesRepository::create_or_reset_process(pool, &id)
        .await
        .expect("create summary process");
    SummaryProcessesRepository::update_process_completed(
        pool,
        &id,
        json!({ "markdown": summary, "english_cache": null }),
        1,
        0.5,
        false,
        &app_lib::summary::refresh::generation_fingerprint(summary),
        &app_lib::summary::refresh::speaker_names_fingerprint(std::iter::empty()),
    )
    .await
    .expect("complete summary");
    id
}

fn dt(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

#[tokio::test]
async fn prep_brief_synthesizes_and_cites_the_prior_two_occurrences() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let older = occurrence(
        pool,
        "Design Review",
        "2026-06-08T10:00:00Z",
        "series-A",
        "Chose the teal palette.",
    )
    .await;
    let newer = occurrence(
        pool,
        "Design Review",
        "2026-06-15T10:00:00Z",
        "series-A",
        "Approved the new nav.",
    )
    .await;
    // A third, different-title different-series decoy — matches NO arm of the specs/0041
    // union (series key ∪ normalized title ∪ manual link), so it must not be pulled in.
    let _decoy = occurrence(
        pool,
        "Unrelated Sync",
        "2026-06-14T10:00:00Z",
        "series-Z",
        "Unrelated.",
    )
    .await;

    // The target occurrence is 2026-06-22; its two prior series-A occurrences, newest first.
    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        Some("series-A"),
        "Design Review",
        dt("2026-06-22T10:00:00Z"),
        2,
    )
    .await
    .unwrap();
    assert_eq!(
        prior,
        vec![newer.clone(), older.clone()],
        "series-keyed, newest first, decoy excluded"
    );

    // Brief over the pinned prior set with a fake reduce that cites both occurrences.
    let calls = Arc::new(Mutex::new(Vec::new()));
    let llm = fake_llm(
        "**Decisions**\n- Approved the new nav [M1]\n- Teal palette [M2]",
        calls.clone(),
    );
    let cancel = CancellationToken::new();
    let answer = execute_pre_call_prep(pool, &prior, llm, GENEROUS_BUDGET, &cancel, |_: Stage| {})
        .await
        .unwrap();

    // Two sources, in [M#] order, both cited by the brief.
    assert_eq!(answer.sources.len(), 2);
    assert_eq!(answer.sources[0].meeting_id, newer);
    assert_eq!(answer.sources[1].meeting_id, older);
    assert!(
        answer.sources.iter().all(|s| s.cited),
        "both occurrences cited"
    );
    assert!(answer.markdown.contains("[M1]") && answer.markdown.contains("[M2]"));
    // Single-pass: exactly one (reduce) LLM call.
    assert_eq!(calls.lock().unwrap().len(), 1);
}

/// specs/0041 WS4 acceptance: a same-title non-recurring recording from yesterday (NULL
/// series key) and a manually linked ad-hoc recording BOTH surface as priors for a
/// series-keyed upcoming occurrence — the old key-shadows-title behavior is gone.
#[tokio::test]
async fn union_matcher_finds_same_title_and_manually_linked_priors() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Yesterday's ad-hoc recording of "the same meeting" — recorded without a calendar
    // link, so its series key is NULL and only the title arm can find it.
    let adhoc_id = format!("meeting-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, origin) \
         VALUES (?, 'Design Review', '2026-06-21T10:00:00Z', '2026-06-21T10:00:00Z', 'recorded')",
    )
    .bind(&adhoc_id)
    .execute(pool)
    .await
    .unwrap();
    SummaryProcessesRepository::create_or_reset_process(pool, &adhoc_id)
        .await
        .unwrap();
    SummaryProcessesRepository::update_process_completed(
        pool,
        &adhoc_id,
        json!({ "markdown": "Ad-hoc notes", "english_cache": null }),
        1,
        0.5,
        false,
        &app_lib::summary::refresh::generation_fingerprint("Ad-hoc notes"),
        &app_lib::summary::refresh::speaker_names_fingerprint(std::iter::empty()),
    )
    .await
    .unwrap();

    // A differently-titled recording, reachable only through a manual link into series-A.
    let linked = occurrence(
        pool,
        "Chat with the design team",
        "2026-06-19T10:00:00Z",
        "ignored-key",
        "Linked notes.",
    )
    .await;
    // Overwrite its calendar key to NULL so only the manual link can match it.
    sqlx::query("UPDATE meetings SET calendar_series_key = NULL WHERE id = ?")
        .bind(&linked)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, 'series-A')")
        .bind(&linked)
        .execute(pool)
        .await
        .unwrap();

    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        Some("series-A"),
        "Design Review",
        dt("2026-06-22T10:00:00Z"),
        5,
    )
    .await
    .unwrap();
    assert_eq!(
        prior,
        vec![adhoc_id, linked],
        "title arm + manual-link arm both contribute, newest first"
    );
}

#[tokio::test]
async fn first_occurrence_has_no_prior_history() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let _only = occurrence(
        pool,
        "Kickoff",
        "2026-06-20T10:00:00Z",
        "series-K",
        "First one.",
    )
    .await;

    // A brand-new series (or the first occurrence) has nothing before it.
    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        Some("series-K"),
        "Kickoff",
        dt("2026-06-20T10:00:00Z"),
        2,
    )
    .await
    .unwrap();
    assert!(
        prior.is_empty(),
        "the first occurrence has no prior history to brief from"
    );
}

#[tokio::test]
async fn carryover_open_items_span_the_prior_occurrences() {
    use app_lib::database::repositories::action_item::ActionItemsRepository;
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let o1 = occurrence(pool, "Weekly", "2026-06-08T10:00:00Z", "S", "a").await;
    let o2 = occurrence(pool, "Weekly", "2026-06-15T10:00:00Z", "S", "b").await;

    // Raw-insert action items: o1 has one open (mine) + one completed; o2 has one open (others).
    let insert = |id: &str, meeting: &str, mine: i64, status: &str, desc: &str| {
        let (id, meeting, status, desc) = (
            id.to_string(),
            meeting.to_string(),
            status.to_string(),
            desc.to_string(),
        );
        async move {
            sqlx::query(
                "INSERT INTO action_items (id, meeting_id, description, assignee_is_self, status, source, user_edited, content_key, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, 'extracted', 0, ?, '2026-06-15T10:00:00Z', '2026-06-15T10:00:00Z')",
            )
            .bind(&id).bind(&meeting).bind(&desc).bind(mine).bind(&status).bind(&id)
            .execute(pool).await.unwrap();
        }
    };
    insert("ai-1", &o1, 1, "open", "I owe the roadmap").await;
    insert("ai-2", &o1, 0, "completed", "done thing").await;
    insert("ai-3", &o2, 0, "open", "Priya owes metrics").await;

    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        Some("S"),
        "Weekly",
        dt("2026-06-22T10:00:00Z"),
        2,
    )
    .await
    .unwrap();
    let open = ActionItemsRepository::get_open_for_meetings(pool, &prior)
        .await
        .unwrap();
    assert_eq!(
        open.len(),
        2,
        "two open items across the series; completed excluded"
    );
    assert!(open[0].assignee_is_self, "mine sorts first");
}

#[tokio::test]
async fn brief_cache_roundtrips_ready_and_status() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    // meeting_briefs FK → meetings; mint a scheduled target row.
    let target = MeetingsRepository::upsert_scheduled_meeting(
        pool,
        "evt-9",
        Some("S"),
        "Weekly",
        dt("2026-06-29T10:00:00Z"),
    )
    .await
    .unwrap()
    .into_id();

    assert!(MeetingBriefsRepository::get(pool, &target)
        .await
        .unwrap()
        .is_none());

    let sources = vec![SourceMeeting {
        meeting_id: "m-prior".into(),
        title: "Weekly".into(),
        created_at: "2026-06-22T10:00:00Z".into(),
        cited: true,
    }];
    let sources_json = serde_json::to_string(&sources).unwrap();
    MeetingBriefsRepository::upsert_ready(
        pool,
        &target,
        "**Decisions**\n- x [M1]",
        &sources_json,
        "fp-1",
        "ollama",
        "llama3",
    )
    .await
    .unwrap();

    let row = MeetingBriefsRepository::get(pool, &target)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "ready");
    assert_eq!(row.source_fingerprint.as_deref(), Some("fp-1"));
    assert!(row.brief_markdown.unwrap().contains("[M1]"));
    let parsed: Vec<SourceMeeting> = serde_json::from_str(&row.sources_json.unwrap()).unwrap();
    assert_eq!(parsed.len(), 1);
    assert!(parsed[0].cited);

    // Status transition to 'none' keeps a fingerprint so it isn't recomputed.
    MeetingBriefsRepository::upsert_status(pool, &target, "none", Some("none"))
        .await
        .unwrap();
    let row = MeetingBriefsRepository::get(pool, &target)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "none");
}
