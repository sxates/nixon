//! specs/0089 Task 6: the post-summary topic jobs against a real migrated DB and a fake LLM.

mod common;

use app_lib::topics::jobs::{self, JobCall};
use app_lib::topics::ledger;
use app_lib::topics::links;
use app_lib::topics::repo::TopicsRepository as Topics;
use common::fresh_db;
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};

const NOW: &str = "2026-01-01T00:00:00Z";

async fn meeting(pool: &SqlitePool, id: &str, title: &str, at: &str, key: Option<&str>) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, calendar_series_key)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(title)
    .bind(at)
    .bind(at)
    .bind(key)
    .execute(pool)
    .await
    .unwrap();
    for (n, text) in [
        "We talked about the Atlas CRM rebuild schedule.",
        "The import step slips a week. Beta launch and Gamma were also mentioned.",
    ]
    .iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time)
             VALUES (?, ?, ?, 'x', ?)",
        )
        .bind(format!("{id}-t{n}"))
        .bind(id)
        .bind(text)
        .bind(n as f64 * 40.0)
        .execute(pool)
        .await
        .unwrap();
    }
}

const ENTRY: &str = r#"{"bullets":[{"text":"Import slips a week","t":"00:40"}],"decisions":[{"text":"Ship import in March","t":"00:00"}],"revises":[]}"#;
const MENTIONS: &str =
    r#"{"mentions":[{"topic":1,"summary":"They discussed the schedule","t":"00:00"}]}"#;

type Calls = Arc<Mutex<Vec<(JobCall, String)>>>;

/// Entry calls answer `entry`, mention calls answer `mention`; every prompt is recorded.
fn fake(
    entry: &'static str,
    mention: &'static str,
    calls: Calls,
) -> impl Fn(JobCall, String, String) -> std::future::Ready<Result<String, String>> + Clone {
    move |call, _system, user| {
        calls.lock().unwrap().push((call, user));
        std::future::ready(Ok(match call {
            JobCall::Entry => entry.to_string(),
            JobCall::Mention => mention.to_string(),
        }))
    }
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn linked_meeting_gets_entry_and_revision_suggestion() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    meeting(p, "m1-0", "Sync", NOW, None).await;
    meeting(p, "m2-0", "Sync", "2026-01-08T00:00:00Z", None).await;
    links::link_meeting(p, &t.id, "m1-0").await.unwrap();
    links::link_meeting(p, &t.id, "m2-0").await.unwrap();
    let app = tauri::test::mock_app();
    let calls: Calls = Default::default();
    let r = jobs::run_for_meeting_with(
        app.handle(),
        p,
        "m1-0",
        "summary",
        None,
        fake(ENTRY, r#"{"mentions":[]}"#, calls.clone()),
    )
    .await
    .unwrap();
    assert_eq!((r.entries, r.suggestions), (1, 0));
    // The second meeting revises the first one's decision.
    let dec = ledger::open_decisions(p, &t.id, 30).await.unwrap();
    assert_eq!(dec.len(), 1);
    let revising: &'static str = Box::leak(
        r#"{"bullets":[],"decisions":[],"revises":[{"decision":1,"note":"Now April","t":"00:40"}]}"#
            .to_string()
            .into_boxed_str(),
    );
    let r = jobs::run_for_meeting_with(
        app.handle(),
        p,
        "m2-0",
        "summary",
        None,
        fake(revising, r#"{"mentions":[]}"#, calls),
    )
    .await
    .unwrap();
    assert_eq!((r.entries, r.suggestions), (1, 1));
    let kinds: Vec<(String, String)> = sqlx::query_as("SELECT kind, text FROM topic_suggestions")
        .fetch_all(p)
        .await
        .unwrap();
    assert_eq!(kinds, [("revision".to_string(), "Now April".to_string())]);
}

#[tokio::test]
async fn unlinked_meeting_gets_note_suggestion_only_for_open_unlinked_topics() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let open = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    let closed = Topics::create(p, "Zeta", "").await.unwrap();
    Topics::update(p, &closed.id, None, None, Some("closed"))
        .await
        .unwrap();
    meeting(p, "m1-1", "Ad hoc", NOW, None).await;
    let app = tauri::test::mock_app();
    let calls: Calls = Default::default();
    let r = jobs::run_for_meeting_with(
        app.handle(),
        p,
        "m1-1",
        "summary",
        None,
        fake(ENTRY, MENTIONS, calls.clone()),
    )
    .await
    .unwrap();
    assert_eq!((r.entries, r.suggestions), (0, 1));
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT topic_id, kind FROM topic_suggestions")
            .fetch_all(p)
            .await
            .unwrap();
    assert_eq!(rows, [(open.id.clone(), "note".to_string())]);
    // The closed topic never reached the model.
    let seen = calls.lock().unwrap();
    assert!(seen.iter().all(|(_, u)| !u.contains("Zeta")));
}

#[tokio::test]
async fn rerun_after_edit_and_dismiss_changes_nothing() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let linked = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    let beta = Topics::create(p, "Beta launch", "").await.unwrap();
    meeting(p, "m1-2", "Sync", NOW, None).await;
    links::link_meeting(p, &linked.id, "m1-2").await.unwrap();
    let app = tauri::test::mock_app();
    // Mentions topics 1 and 2; whichever indexes exist are used, the rest are dropped.
    let mentions: &'static str = r#"{"mentions":[{"topic":1,"summary":"It came up","t":"00:00"},{"topic":2,"summary":"It came up","t":"00:00"}]}"#;
    let run = || {
        jobs::run_for_meeting_with(
            app.handle(),
            p,
            "m1-2",
            "summary",
            None,
            fake(ENTRY, mentions, Default::default()),
        )
    };
    let r = run().await.unwrap();
    assert_eq!((r.entries, r.suggestions), (1, 1));

    // The owner edits the entry and dismisses the suggestion.
    let entries = ledger::entries_for_topic(p, &linked.id).await.unwrap();
    let mut body = entries[0].body.clone();
    body.bullets[0].text = "Owner wording".into();
    ledger::edit_entry(p, &entries[0].id, body).await.unwrap();
    sqlx::query("UPDATE topic_suggestions SET status = 'dismissed'")
        .execute(p)
        .await
        .unwrap();
    let snapshot = || async {
        sqlx::query_as::<_, (String, String)>("SELECT body_json, ai_body_json FROM topic_entries")
            .fetch_all(p)
            .await
            .unwrap()
    };
    let before = snapshot().await;

    // Summary regenerated: same fingerprint, nothing happens.
    let r = run().await.unwrap();
    assert_eq!((r.entries, r.suggestions), (0, 0));
    assert_eq!(snapshot().await, before);

    // A new topic changes the fingerprint; the dismissed suggestion must not come back.
    Topics::create(p, "Gamma", "").await.unwrap();
    let r = run().await.unwrap();
    assert_eq!(r.entries, 0);
    assert_eq!(snapshot().await, before);
    let beta_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM topic_suggestions WHERE topic_id = ?")
            .bind(&beta.id)
            .fetch_one(p)
            .await
            .unwrap();
    assert_eq!(beta_rows, 1, "the dismissed suggestion is not re-created");
    let pending_beta: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topic_suggestions WHERE topic_id = ? AND status = 'pending'",
    )
    .bind(&beta.id)
    .fetch_one(p)
    .await
    .unwrap();
    assert_eq!(pending_beta, 0);
}

#[tokio::test]
async fn job_writes_nothing_when_topic_closed_mid_run() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    meeting(p, "m1-3", "Sync", NOW, None).await;
    links::link_meeting(p, &t.id, "m1-3").await.unwrap();
    let app = tauri::test::mock_app();
    let pool = p.clone();
    let tid = t.id.clone();
    let llm = move |_c: JobCall, _s: String, _u: String| {
        let pool = pool.clone();
        let tid = tid.clone();
        async move {
            sqlx::query("UPDATE topics SET status = 'closed' WHERE id = ?")
                .bind(&tid)
                .execute(&pool)
                .await
                .unwrap();
            Ok::<String, String>(ENTRY.to_string())
        }
    };
    let r = jobs::run_for_meeting_with(app.handle(), p, "m1-3", "summary", None, llm)
        .await
        .unwrap();
    assert_eq!((r.entries, r.suggestions), (0, 0));
    assert_eq!(count(p, "SELECT COUNT(*) FROM topic_entries").await, 0);
    assert_eq!(count(p, "SELECT COUNT(*) FROM topic_suggestions").await, 0);
}

#[tokio::test]
async fn backfill_processes_oldest_first_and_later_meeting_sees_earlier_decision() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    meeting(p, "old-4", "Sync", "2026-01-01T00:00:00Z", Some("k")).await;
    meeting(p, "new-4", "Sync", "2026-01-08T00:00:00Z", Some("k")).await;
    links::link_series_of(p, &t.id, "new-4").await.unwrap();
    let app = tauri::test::mock_app();
    let calls: Calls = Default::default();
    let llm = fake(ENTRY, r#"{"mentions":[]}"#, calls.clone());
    let ids = vec!["old-4".to_string(), "new-4".to_string()];
    let r = jobs::backfill_with(
        app.handle(),
        p,
        &ids,
        |_id| async { Ok(("summary".to_string(), None)) },
        llm,
    )
    .await;
    assert_eq!(r.entries, 2);
    let seen = calls.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(!seen[0].1.contains("Ship import in March"));
    assert!(
        seen[1].1.contains("Ship import in March"),
        "second prompt sees the first decision"
    );
    let order: Vec<String> =
        sqlx::query_scalar("SELECT meeting_id FROM topic_entries ORDER BY created_at, id")
            .fetch_all(p)
            .await
            .unwrap();
    assert_eq!(order, ["old-4", "new-4"]);
    // Backfill never detects.
    assert!(seen.iter().all(|(c, _)| *c == JobCall::Entry));
}

#[tokio::test]
async fn pretag_only_when_exactly_one_topic() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let a = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m1-5", "Sync", NOW, None).await;
    for (id, triage) in [("ai-1", "suggested"), ("ai-2", "listed")] {
        sqlx::query(
            "INSERT INTO action_items (id, meeting_id, description, content_key, status, triage, created_at, updated_at)
             VALUES (?, 'm1-5', 'do it', ?, 'open', ?, ?, ?)",
        )
        .bind(id).bind(id).bind(triage).bind(NOW).bind(NOW)
        .execute(p).await.unwrap();
    }
    // No topic matches: nothing.
    assert_eq!(jobs::pretag_suggestions(p, "m1-5").await.unwrap(), 0);
    links::link_meeting(p, &a.id, "m1-5").await.unwrap();
    let b = Topics::create(p, "Beta", "").await.unwrap();
    links::link_meeting(p, &b.id, "m1-5").await.unwrap();
    assert_eq!(
        jobs::pretag_suggestions(p, "m1-5").await.unwrap(),
        0,
        "two topics: ambiguous"
    );
    assert_eq!(
        count(
            p,
            "SELECT COUNT(*) FROM action_items WHERE topic_id IS NOT NULL"
        )
        .await,
        0
    );
    sqlx::query("UPDATE topics SET status = 'closed' WHERE id = ?")
        .bind(&b.id)
        .execute(p)
        .await
        .unwrap();
    assert_eq!(jobs::pretag_suggestions(p, "m1-5").await.unwrap(), 1);
    let tagged: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, topic_id FROM action_items ORDER BY id")
            .fetch_all(p)
            .await
            .unwrap();
    assert_eq!(
        tagged,
        [
            ("ai-1".to_string(), Some(a.id.clone())),
            ("ai-2".to_string(), None)
        ]
    );
}

#[tokio::test]
async fn linked_meeting_without_transcript_still_gets_an_entry() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES ('notes-only-7', 'Imported', ?, ?)")
        .bind(NOW).bind(NOW).execute(p).await.unwrap();
    links::link_meeting(p, &t.id, "notes-only-7").await.unwrap();
    let app = tauri::test::mock_app();
    let r = jobs::run_for_meeting_with(
        app.handle(),
        p,
        "notes-only-7",
        "summary",
        Some("my notes"),
        fake(ENTRY, r#"{"mentions":[]}"#, Default::default()),
    )
    .await
    .unwrap();
    assert_eq!(r.entries, 1);
    // Cited times can't resolve without evidence labels.
    let body: String = sqlx::query_scalar("SELECT body_json FROM topic_entries")
        .fetch_one(p)
        .await
        .unwrap();
    assert!(!body.contains("\"t\""), "{body}");
}
