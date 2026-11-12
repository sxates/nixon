//! specs/0088 W1: to-do / inbox / done views, triage transitions, detail, cleanup.

mod common;

use app_lib::action_items::diff::ResolvedCandidate;
use app_lib::database::repositories::action_item::ActionItemsRepository;
use app_lib::database::repositories::action_item_todo::{
    ActionItemTodoRepository as Todo, TodoView,
};
use chrono::{Duration, Utc};
use common::fresh_db;
use sqlx::SqlitePool;

fn cand(desc: &str) -> ResolvedCandidate {
    ResolvedCandidate {
        description: desc.to_string(),
        assignee_person_id: None,
        assignee_is_self: false,
        assignee_raw: None,
        due_hint: None,
        due_date: None,
    }
}

async fn meeting(pool: &SqlitePool, id: &str, days_ago: i64) {
    let ts = (Utc::now() - Duration::days(days_ago)).to_rfc3339();
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(format!("Meeting {id}"))
        .bind(&ts)
        .bind(&ts)
        .execute(pool)
        .await
        .unwrap();
}

async fn source(pool: &SqlitePool, item: &str, meeting_id: &str) {
    sqlx::query(
        "INSERT OR IGNORE INTO action_item_sources (action_item_id, meeting_id, context, phrasing, added_at)
         VALUES (?, ?, 'ctx', 'phr', '2026-01-01T00:00:00Z')",
    )
    .bind(item)
    .bind(meeting_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Extracted row (no source row, as before Task 6) with explicit triage/status.
async fn extracted(
    pool: &SqlitePool,
    meeting_id: &str,
    desc: &str,
    triage: &str,
    status: &str,
) -> String {
    let item =
        ActionItemsRepository::create(pool, Some(meeting_id), &cand(desc), "extracted", desc)
            .await
            .unwrap();
    sqlx::query("UPDATE action_items SET triage = ?, status = ? WHERE id = ?")
        .bind(triage)
        .bind(status)
        .bind(&item.id)
        .execute(pool)
        .await
        .unwrap();
    item.id
}

async fn ids(pool: &SqlitePool, v: TodoView) -> Vec<String> {
    Todo::list_view(pool, v)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect()
}

#[tokio::test]
async fn list_view_partitions_rows() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    let listed = ActionItemsRepository::create(pool, None, &cand("manual"), "manual", "manual")
        .await
        .unwrap()
        .id;
    let done = extracted(pool, "m1", "done", "listed", "completed").await;
    let inbox = extracted(pool, "m1", "sugg", "suggested", "open").await;
    let dism = extracted(pool, "m1", "dism", "suggested", "dismissed").await;
    let rec = extracted(pool, "m1", "rec", "record", "open").await;
    assert_eq!(ids(pool, TodoView::List).await, vec![listed]);
    assert_eq!(ids(pool, TodoView::Done).await, vec![done]);
    assert_eq!(ids(pool, TodoView::Inbox).await, vec![inbox]);
    assert_eq!(ids(pool, TodoView::Dismissed).await, vec![dism]);
    assert!(!ids(pool, TodoView::Inbox).await.contains(&rec));
    assert_eq!(Todo::inbox_count(pool).await.unwrap(), 1);
}

#[tokio::test]
async fn add_to_list_appends_and_keeps_assignee() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    sqlx::query("INSERT INTO people (id, display_name, created_at, updated_at) VALUES ('p-1','Pat','2026-01-01','2026-01-01')")
        .execute(pool).await.ok();
    for d in ["a", "b"] {
        ActionItemsRepository::create(pool, None, &cand(d), "manual", d)
            .await
            .unwrap();
    }
    let rec = extracted(pool, "m1", "rec", "record", "open").await;
    sqlx::query(
        "UPDATE action_items SET assignee_person_id = NULL, assignee_raw = 'Pat' WHERE id = ?",
    )
    .bind(&rec)
    .execute(pool)
    .await
    .unwrap();
    let item = Todo::add_to_list(pool, &rec).await.unwrap().unwrap();
    assert_eq!(item.sort_order, Some(2));
    assert_eq!(item.triage, "listed");
    assert_eq!(item.assignee_raw.as_deref(), Some("Pat"));
    assert!(item.user_edited && item.adopted_at.is_some());
}

#[tokio::test]
async fn complete_then_uncheck_restores_position() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let mut v = vec![];
    for d in ["a", "b", "c"] {
        v.push(
            ActionItemsRepository::create(pool, None, &cand(d), "manual", d)
                .await
                .unwrap()
                .id,
        );
    }
    ActionItemsRepository::set_status(pool, &v[1], "completed")
        .await
        .unwrap();
    assert_eq!(
        ids(pool, TodoView::List).await,
        vec![v[0].clone(), v[2].clone()]
    );
    let back = ActionItemsRepository::set_status(pool, &v[1], "open")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.sort_order, Some(1));
    assert_eq!(ids(pool, TodoView::List).await, v);
}

#[tokio::test]
async fn reassign_moves_suggestion_to_record() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    let a = extracted(pool, "m1", "a", "suggested", "open").await;
    let b = extracted(pool, "m1", "b", "suggested", "open").await;
    sqlx::query("UPDATE action_items SET assignee_raw = 'Someone' WHERE id = ?")
        .bind(&a)
        .execute(pool)
        .await
        .unwrap();
    Todo::reconcile_triage_after_assignee_change(pool, &a)
        .await
        .unwrap();
    Todo::reconcile_triage_after_assignee_change(pool, &b)
        .await
        .unwrap();
    assert_eq!(
        ActionItemsRepository::get(pool, &a)
            .await
            .unwrap()
            .unwrap()
            .triage,
        "record"
    );
    assert_eq!(
        ActionItemsRepository::get(pool, &b)
            .await
            .unwrap()
            .unwrap()
            .triage,
        "suggested"
    );
}

#[tokio::test]
async fn dismiss_older_than_uses_latest_source() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "old", 40).await;
    meeting(pool, "new", 5).await;
    let both = extracted(pool, "old", "both", "suggested", "open").await;
    source(pool, &both, "old").await;
    source(pool, &both, "new").await;
    let single = extracted(pool, "old", "single", "suggested", "open").await;
    source(pool, &single, "old").await;
    // No source rows yet (pre-Task 6): falls back to the item's own meeting.
    let bare = extracted(pool, "old", "bare", "suggested", "open").await;
    sqlx::query("DELETE FROM action_item_sources WHERE action_item_id = ?")
        .bind(&bare)
        .execute(pool)
        .await
        .unwrap();
    let bare_new = extracted(pool, "new", "bare-new", "suggested", "open").await;
    let mut got = Todo::dismiss_suggestions_older_than(pool, 30)
        .await
        .unwrap();
    got.sort();
    let mut want = vec![single.clone(), bare.clone()];
    want.sort();
    assert_eq!(got, want);
    for id in [&both, &bare_new] {
        assert_eq!(
            ActionItemsRepository::get(pool, id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "open"
        );
    }
    let s = ActionItemsRepository::get(pool, &single)
        .await
        .unwrap()
        .unwrap();
    assert!(s.status == "dismissed" && s.user_edited);
}

#[tokio::test]
async fn meeting_page_includes_attached_items() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "A", 3).await;
    meeting(pool, "B", 1).await;
    let x = extracted(pool, "A", "x", "suggested", "open").await;
    source(pool, &x, "B").await;
    let on_b: Vec<_> = Todo::list_for_meeting_page(pool, "B")
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(on_b, vec![x]);
}

#[tokio::test]
async fn detail_lists_sources_oldest_first() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "new", 1).await;
    meeting(pool, "old", 9).await;
    let x = extracted(pool, "old", "x", "suggested", "open").await;
    source(pool, &x, "new").await;
    source(pool, &x, "old").await;
    sqlx::query("INSERT INTO action_item_people (action_item_id, person_id) VALUES (?, 'p-9')")
        .bind(&x)
        .execute(pool)
        .await
        .unwrap();
    let d = Todo::get_detail(pool, &x).await.unwrap().unwrap();
    let order: Vec<_> = d.sources.iter().map(|s| s.meeting_id.as_str()).collect();
    assert_eq!(order, vec!["old", "new"]);
    assert_eq!(d.person_ids, vec!["p-9"]);
    let rows = Todo::list_view(pool, TodoView::Inbox).await.unwrap();
    assert_eq!(rows[0].source_count, 2);
    assert_eq!(rows[0].latest_context.as_deref(), Some("ctx"));
    assert!(Todo::set_notes(pool, &x, Some("hi")).await.unwrap());
    assert!(!Todo::set_notes(pool, "nope", Some("hi")).await.unwrap());
}
