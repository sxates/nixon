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
        ..Default::default()
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
async fn adopt_appends_and_keeps_assignee() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    sqlx::query("INSERT INTO people (id, display_name, created_at, updated_at) VALUES ('p-1','Pat','2026-01-01','2026-01-01')")
        .execute(pool)
        .await
        .unwrap();
    for d in ["a", "b"] {
        ActionItemsRepository::create(pool, None, &cand(d), "manual", d)
            .await
            .unwrap();
    }
    let rec = extracted(pool, "m1", "rec", "record", "open").await;
    sqlx::query(
        "UPDATE action_items SET assignee_person_id = 'p-1', assignee_raw = NULL WHERE id = ?",
    )
    .bind(&rec)
    .execute(pool)
    .await
    .unwrap();
    let item = Todo::adopt(pool, &rec).await.unwrap().unwrap();
    assert_eq!(item.sort_order, Some(2));
    assert_eq!(item.triage, "listed");
    assert_eq!(item.assignee_person_id.as_deref(), Some("p-1"));
    assert!(item.user_edited && item.adopted_at.is_some());
}

#[tokio::test]
async fn add_to_list_makes_the_item_mine() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    let unassigned = extracted(pool, "m1", "nobody", "suggested", "open").await;
    let raw = extracted(pool, "m1", "raw", "suggested", "open").await;
    sqlx::query("UPDATE action_items SET assignee_raw = 'Someone' WHERE id = ?")
        .bind(&raw)
        .execute(pool)
        .await
        .unwrap();
    for id in [&unassigned, &raw] {
        let item = Todo::add_to_list(pool, id).await.unwrap().unwrap();
        assert_eq!(item.triage, "listed");
        assert!(item.assignee_is_self);
        assert!(item.assignee_person_id.is_none() && item.assignee_raw.is_none());
    }
}

#[tokio::test]
async fn manual_create_is_mine() {
    let (_d, db) = fresh_db().await;
    let item = ActionItemsRepository::create(db.pool(), None, &cand("typed"), "manual", "typed")
        .await
        .unwrap();
    assert!(item.assignee_is_self);
}

#[tokio::test]
async fn reconcile_moves_record_back_to_suggested_when_mine_or_nobody() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    let mine = extracted(pool, "m1", "a", "record", "open").await;
    let nobody = extracted(pool, "m1", "b", "record", "open").await;
    let done = extracted(pool, "m1", "c", "record", "completed").await;
    sqlx::query("UPDATE action_items SET assignee_is_self = 1 WHERE id = ?")
        .bind(&mine)
        .execute(pool)
        .await
        .unwrap();
    for id in [&mine, &nobody, &done] {
        Todo::reconcile_triage_after_assignee_change(pool, id)
            .await
            .unwrap();
    }
    let triage = |id: String| async move {
        ActionItemsRepository::get(pool, &id)
            .await
            .unwrap()
            .unwrap()
            .triage
    };
    assert_eq!(triage(mine).await, "suggested");
    assert_eq!(triage(nobody).await, "suggested");
    assert_eq!(triage(done).await, "record");
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

#[tokio::test]
async fn restore_reopens_dismissed_item() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    let id = extracted(pool, "m1", "x", "suggested", "dismissed").await;
    assert_eq!(ids(pool, TodoView::Dismissed).await, vec![id.clone()]);
    let item = Todo::restore(pool, &id).await.unwrap().unwrap();
    assert!(item.status == "open" && item.user_edited);
    assert_eq!(ids(pool, TodoView::Inbox).await, vec![id]);
    assert!(ids(pool, TodoView::Dismissed).await.is_empty());
}

#[tokio::test]
async fn dismissed_view_excludes_record_rows() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    extracted(pool, "m1", "rec", "record", "dismissed").await;
    assert!(ids(pool, TodoView::Dismissed).await.is_empty());
}

#[tokio::test]
async fn standalone_listed_row_appears_in_list() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let id = ActionItemsRepository::create(pool, None, &cand("solo"), "manual", "solo")
        .await
        .unwrap()
        .id;
    let rows = Todo::list_view(pool, TodoView::List).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item.id, id);
    assert_eq!(rows[0].source_count, 0);
    assert!(rows[0].latest_source_at.is_none());
}

#[tokio::test]
async fn inbox_orders_by_latest_source_newest_first() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "older", 20).await;
    meeting(pool, "newer", 2).await;
    let a = extracted(pool, "older", "a", "suggested", "open").await;
    let b = extracted(pool, "newer", "b", "suggested", "open").await;
    assert_eq!(ids(pool, TodoView::Inbox).await, vec![b, a]);
}

async fn person(pool: &SqlitePool, id: &str, name: &str) {
    sqlx::query(
        "INSERT INTO people (id, display_name, created_at, updated_at) VALUES (?, ?, 'now', 'now')",
    )
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

async fn mention_ids(pool: &SqlitePool, item: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT person_id FROM action_item_people WHERE action_item_id = ? ORDER BY person_id",
    )
    .bind(item)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn create_with_mention_syncs_people() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    person(pool, "p-1", "Sam Lee").await;
    let desc = "Check in with @[Sam Lee](person:p-1)";
    let id = ActionItemsRepository::create(pool, None, &cand(desc), "manual", "k")
        .await
        .unwrap()
        .id;
    Todo::sync_mentions(pool, &id, desc).await.unwrap();
    assert_eq!(mention_ids(pool, &id).await, vec!["p-1".to_string()]);
}

#[tokio::test]
async fn edit_removing_mention_clears_row() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    person(pool, "p-1", "Sam Lee").await;
    let id = ActionItemsRepository::create(pool, None, &cand("x"), "manual", "k")
        .await
        .unwrap()
        .id;
    Todo::sync_mentions(pool, &id, "hi @[Sam Lee](person:p-1)")
        .await
        .unwrap();
    Todo::sync_mentions(pool, &id, "hi Sam").await.unwrap();
    assert!(mention_ids(pool, &id).await.is_empty());
}

#[tokio::test]
async fn person_delete_removes_mentions_keeps_title() {
    use app_lib::database::repositories::people::PeopleRepository;
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    person(pool, "p-1", "Sam Lee").await;
    let desc = "Ping @[Sam Lee](person:p-1)";
    let id = ActionItemsRepository::create(pool, None, &cand(desc), "manual", "k")
        .await
        .unwrap()
        .id;
    Todo::sync_mentions(pool, &id, desc).await.unwrap();
    PeopleRepository::delete(pool, "p-1").await.unwrap();
    assert!(mention_ids(pool, &id).await.is_empty());
    let item = ActionItemsRepository::get(pool, &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(item.description, desc);
}

async fn delete_meeting(pool: &SqlitePool, id: &str) {
    use app_lib::database::repositories::meeting::MeetingsRepository;
    assert!(MeetingsRepository::delete_meeting(pool, id).await.unwrap());
}

async fn meeting_id_of(pool: &SqlitePool, item: &str) -> Option<Option<String>> {
    sqlx::query_scalar("SELECT meeting_id FROM action_items WHERE id = ?")
        .bind(item)
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn meeting_delete_keeps_listed_as_standalone() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 5).await;
    let id = extracted(pool, "m1", "keep me", "listed", "open").await;
    source(pool, &id, "m1").await;
    delete_meeting(pool, "m1").await;
    assert_eq!(meeting_id_of(pool, &id).await, Some(None));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_item_sources")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn meeting_delete_repoints_multi_source_item() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 30).await;
    meeting(pool, "m2", 20).await;
    meeting(pool, "m3", 10).await;
    let id = extracted(pool, "m3", "shared", "suggested", "open").await;
    source(pool, &id, "m3").await;
    source(pool, &id, "m2").await;
    source(pool, &id, "m1").await;
    delete_meeting(pool, "m3").await;
    // Oldest remaining source meeting wins.
    assert_eq!(meeting_id_of(pool, &id).await, Some(Some("m1".to_string())));
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM action_item_sources WHERE action_item_id = ? AND meeting_id = 'm3'",
    )
    .bind(&id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn meeting_delete_removes_single_source_suggestion() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 5).await;
    let id = extracted(pool, "m1", "only here", "suggested", "open").await;
    source(pool, &id, "m1").await;
    person(pool, "p-1", "Sam Lee").await;
    sqlx::query("INSERT INTO action_item_people (action_item_id, person_id) VALUES (?, 'p-1')")
        .bind(&id)
        .execute(pool)
        .await
        .unwrap();
    delete_meeting(pool, "m1").await;
    assert_eq!(meeting_id_of(pool, &id).await, None);
    assert!(mention_ids(pool, &id).await.is_empty());
}

#[tokio::test]
async fn tracked_for_merge_orders_groups_strips_tokens_and_windows_completed() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m1", 1).await;
    let sugg = extracted(pool, "m1", "Suggested one", "suggested", "open").await;
    let listed = extracted(
        pool,
        "m1",
        "Check in with @[Sam](person:p-1)",
        "listed",
        "open",
    )
    .await;
    let dismissed = extracted(pool, "m1", "Dismissed one", "suggested", "dismissed").await;
    let record = extracted(pool, "m1", "Someone else's", "record", "dismissed").await;
    let fresh = extracted(pool, "m1", "Finished lately", "listed", "completed").await;
    let stale = extracted(pool, "m1", "Finished ages ago", "listed", "completed").await;
    for (id, days) in [(&fresh, 5), (&stale, 90)] {
        sqlx::query("UPDATE action_items SET completed_at = ? WHERE id = ?")
            .bind((Utc::now() - Duration::days(days)).to_rfc3339())
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
    let done_record = extracted(pool, "m1", "Others finished", "record", "completed").await;
    sqlx::query("UPDATE action_items SET completed_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(&done_record)
        .execute(pool)
        .await
        .unwrap();
    let got = Todo::tracked_for_merge(pool).await.unwrap();
    let got_ids: Vec<&str> = got.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        got_ids,
        [
            listed.as_str(),
            sugg.as_str(),
            dismissed.as_str(),
            fresh.as_str()
        ]
    );
    assert!(!got_ids.contains(&record.as_str()) && !got_ids.contains(&stale.as_str()));
    assert!(!got_ids.contains(&done_record.as_str()));
    assert_eq!(got[0].description, "Check in with Sam");
    assert_eq!(got[0].meeting_id.as_deref(), Some("m1"));
}

#[tokio::test]
async fn highlight_round_trips_on_the_list_view_and_rejects_unknown_colours() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let item = ActionItemsRepository::create(pool, None, &cand("typed"), "manual", "typed")
        .await
        .unwrap();
    let highlight = |rows: Vec<app_lib::database::repositories::action_item_todo::TodoRow>| {
        rows.into_iter().find(|r| r.item.id == item.id).unwrap().highlight
    };

    assert!(Todo::set_highlight(pool, &item.id, Some("yellow")).await.unwrap());
    assert_eq!(highlight(Todo::list_view(pool, TodoView::List).await.unwrap()), Some("yellow".into()));
    // A highlight is not a content edit: re-extraction protection stays as it was.
    let after = ActionItemsRepository::get(pool, &item.id).await.unwrap().unwrap();
    assert_eq!(after.updated_at, item.updated_at);

    assert!(Todo::set_highlight(pool, &item.id, Some("purple")).await.is_err());
    assert!(Todo::set_highlight(pool, &item.id, None).await.unwrap());
    assert_eq!(highlight(Todo::list_view(pool, TodoView::List).await.unwrap()), None);
    assert!(!Todo::set_highlight(pool, "ai-missing", None).await.unwrap());
}
