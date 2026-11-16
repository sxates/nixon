//! specs/0090: topic order + stars + "Keep off the To-do list", and subset reorder of
//! to-dos within the slots they already hold.

mod common;

use std::borrow::Cow;

use app_lib::action_items::diff::ResolvedCandidate;
use app_lib::database::repositories::action_item::ActionItemsRepository as Items;
use app_lib::database::repositories::action_item_todo::{
    ActionItemTodoRepository as Todo, TodoView,
};
use app_lib::topics::repo::{TopicPatch, TopicsRepository as Topics};
use common::fresh_db;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

const V: i64 = 20261116000000;

// ---- migration backfill -------------------------------------------------------------

async fn raw_topic(pool: &SqlitePool, id: &str, name: &str, status: &str) {
    sqlx::query(
        "INSERT INTO topics (id, name, status, created_at, updated_at)
         VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(name)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

async fn raw_entry(pool: &SqlitePool, id: &str, topic: &str, at: &str) {
    sqlx::query(
        "INSERT INTO topic_entries (id, topic_id, meeting_id, kind, occurred_at, body_json, created_at)
         VALUES (?, ?, ?, 'manual', ?, '{}', ?)",
    )
    .bind(id)
    .bind(topic)
    .bind(format!("m-{id}"))
    .bind(at)
    .bind(at)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_backfills_order_and_stars_recent_five_open() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = Cow::Owned(
        before
            .migrations
            .iter()
            .filter(|m| m.version < V)
            .cloned()
            .collect(),
    );
    before.run(&pool).await.expect("pre-migrations");

    // Six open topics (two without entries) and one closed, most active of all.
    for (id, name, status) in [
        ("t-a", "alpha", "open"),
        ("t-b", "Bravo", "open"),
        ("t-c", "charlie", "open"),
        ("t-d", "Delta", "open"),
        ("t-e", "echo", "open"),
        ("t-f", "Foxtrot", "open"),
        ("t-z", "Aardvark", "closed"),
    ] {
        raw_topic(&pool, id, name, status).await;
    }
    raw_entry(&pool, "e1", "t-a", "2026-03-01T00:00:00Z").await;
    raw_entry(&pool, "e2", "t-a", "2026-01-01T00:00:00Z").await; // MAX wins, not MIN
    raw_entry(&pool, "e3", "t-b", "2026-02-01T00:00:00Z").await;
    raw_entry(&pool, "e4", "t-c", "2026-04-01T00:00:00Z").await;
    raw_entry(&pool, "e5", "t-d", "2026-01-15T00:00:00Z").await;
    raw_entry(&pool, "e6", "t-z", "2026-12-01T00:00:00Z").await;
    // t-e and t-f have no entries: last; exactly one of them makes the five.

    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let rows: Vec<(String, Option<i64>, bool, bool)> = sqlx::query_as(
        "SELECT id, sort_order, starred, hide_from_todo FROM topics ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    // Open first (by name, case-insensitive), then closed.
    let order: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(order, ["t-a", "t-b", "t-c", "t-d", "t-e", "t-f", "t-z"]);
    let orders: Vec<Option<i64>> = rows.iter().map(|r| r.1).collect();
    assert_eq!(orders, (0..7).map(Some).collect::<Vec<_>>());
    assert!(rows.iter().all(|r| !r.3), "nothing hidden by default");

    let starred: Vec<&str> = rows.iter().filter(|r| r.2).map(|r| r.0.as_str()).collect();
    assert_eq!(starred.len(), 5);
    for id in ["t-a", "t-b", "t-c", "t-d"] {
        assert!(starred.contains(&id), "{id} has entries: starred");
    }
    assert!(!starred.contains(&"t-z"), "closed topics are never starred");
    assert!(
        starred.contains(&"t-e") ^ starred.contains(&"t-f"),
        "one entry-less topic fills the fifth slot"
    );
}

// ---- Topics::list / create / patch / reorder ----------------------------------------

fn names(list: &[app_lib::topics::model::TopicSummary]) -> Vec<&str> {
    list.iter().map(|t| t.name.as_str()).collect()
}

#[tokio::test]
async fn create_appends_and_list_follows_manual_order() {
    let (_d, db) = fresh_db().await;
    let p = db.pool();
    let z = Topics::create(p, "Zeta", "").await.unwrap();
    let a = Topics::create(p, "Atlas", "").await.unwrap();
    let m = Topics::create(p, "Mid", "").await.unwrap();
    assert_eq!(
        [z.sort_order, a.sort_order, m.sort_order],
        [Some(0), Some(1), Some(2)]
    );
    assert!(!z.starred && !z.hide_from_todo);
    // Creation order, not name order.
    assert_eq!(
        names(&Topics::list(p, None).await.unwrap()),
        ["Zeta", "Atlas", "Mid"]
    );

    // Closing doesn't move a topic: open and archived share one order.
    Topics::update(p, &z.id, None, None, Some("closed"))
        .await
        .unwrap();
    assert_eq!(
        names(&Topics::list(p, None).await.unwrap()),
        ["Zeta", "Atlas", "Mid"]
    );

    let n = Topics::reorder(
        p,
        &[
            m.id.clone(),
            z.id.clone(),
            a.id.clone(),
            "topic-gone".into(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(n, 3, "unknown ids are skipped");
    let list = Topics::list(p, None).await.unwrap();
    assert_eq!(names(&list), ["Mid", "Zeta", "Atlas"]);
    assert_eq!(list[0].sort_order, Some(0));
    assert_eq!(
        names(&Topics::list(p, Some("open")).await.unwrap()),
        ["Mid", "Atlas"]
    );

    // A row with no order (pre-backfill shape) sorts last, then by name.
    sqlx::query("UPDATE topics SET sort_order = NULL WHERE id = ?")
        .bind(&m.id)
        .execute(p)
        .await
        .unwrap();
    assert_eq!(
        names(&Topics::list(p, None).await.unwrap()),
        ["Zeta", "Atlas", "Mid"]
    );
    // And the next topic still appends after the max.
    let n2 = Topics::create(p, "New", "").await.unwrap();
    assert_eq!(n2.sort_order, Some(3));
}

#[tokio::test]
async fn patch_sets_star_and_hide_and_leaves_the_rest() {
    let (_d, db) = fresh_db().await;
    let p = db.pool();
    let t = Topics::create(p, "Atlas", "desc").await.unwrap();
    let starred = Topics::patch(
        p,
        &t.id,
        &TopicPatch {
            starred: Some(true),
            ..TopicPatch::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(starred.starred);
    assert!(!starred.hide_from_todo);
    assert_eq!(
        (starred.name.as_str(), starred.description.as_str()),
        ("Atlas", "desc")
    );

    let hidden = Topics::patch(
        p,
        &t.id,
        &TopicPatch {
            hide_from_todo: Some(true),
            name: Some("Atlas 2"),
            ..TopicPatch::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(hidden.starred, "untouched flag kept");
    assert!(hidden.hide_from_todo);
    assert_eq!(hidden.name, "Atlas 2");

    // The legacy update path never touches the flags.
    let closed = Topics::update(p, &t.id, None, None, Some("closed"))
        .await
        .unwrap()
        .unwrap();
    assert!(closed.starred && closed.hide_from_todo);

    let off = Topics::patch(
        p,
        &t.id,
        &TopicPatch {
            starred: Some(false),
            hide_from_todo: Some(false),
            ..TopicPatch::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!off.starred && !off.hide_from_todo);
    let summary = &Topics::list(p, None).await.unwrap()[0];
    assert!(!summary.starred && !summary.hide_from_todo);
    assert!(Topics::patch(p, "nope", &TopicPatch::default())
        .await
        .unwrap()
        .is_none());

    // The wire shape the frontend reads.
    let json = serde_json::to_value(&off).unwrap();
    assert_eq!(json["sortOrder"], 0);
    assert_eq!(json["starred"], false);
    assert_eq!(json["hideFromTodo"], false);
    let json = serde_json::to_value(summary).unwrap();
    assert!(json.get("sortOrder").is_some() && json.get("hideFromTodo").is_some());
}

// ---- reorder_within -------------------------------------------------------------------

async fn item(pool: &SqlitePool, desc: &str, sort: Option<i64>) -> String {
    let c = ResolvedCandidate {
        description: desc.to_string(),
        ..Default::default()
    };
    let it = Items::create(pool, None, &c, "manual", desc).await.unwrap();
    sqlx::query(
        "UPDATE action_items SET triage = 'listed', status = 'open', sort_order = ? WHERE id = ?",
    )
    .bind(sort)
    .bind(&it.id)
    .execute(pool)
    .await
    .unwrap();
    it.id
}

async fn global(pool: &SqlitePool) -> Vec<(String, Option<i64>)> {
    sqlx::query_as(
        "SELECT description, sort_order FROM action_items
         ORDER BY sort_order IS NULL, sort_order, created_at",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn descs(g: &[(String, Option<i64>)]) -> Vec<&str> {
    g.iter().map(|r| r.0.as_str()).collect()
}

#[tokio::test]
async fn reorder_within_swaps_only_the_subsets_slots() {
    let (_d, db) = fresh_db().await;
    let p = db.pool();
    let _x1 = item(p, "X1", Some(0)).await;
    let t1 = item(p, "T1", Some(1)).await;
    let _y1 = item(p, "Y1", Some(2)).await;
    let t2 = item(p, "T2", Some(3)).await;
    let t3 = item(p, "T3", Some(4)).await;

    let n = Items::reorder_within(p, &[t3.clone(), t1.clone(), t2.clone()])
        .await
        .unwrap();
    assert_eq!(n, 3);
    let g = global(p).await;
    assert_eq!(descs(&g), ["X1", "T3", "Y1", "T1", "T2"]);
    // X1 and Y1 kept their exact values.
    assert_eq!(g[0].1, Some(0));
    assert_eq!(g[2].1, Some(2));
}

#[tokio::test]
async fn reorder_within_gives_null_slots_values_past_the_max() {
    let (_d, db) = fresh_db().await;
    let p = db.pool();
    let _x = item(p, "X", Some(10)).await;
    let a = item(p, "A", Some(3)).await;
    let b = item(p, "B", None).await;
    let c = item(p, "C", None).await;

    // New order C, A, B: slots {3, NULL->11 (C), NULL->12 (B)}... assigned in the GIVEN
    // order (C first gets 11, B gets 12), then sorted [3, 11, 12] -> C=3, A=11, B=12.
    Items::reorder_within(p, &[c.clone(), a.clone(), b.clone()])
        .await
        .unwrap();
    let g = global(p).await;
    assert_eq!(
        g,
        [
            ("C".to_string(), Some(3)),
            ("X".to_string(), Some(10)),
            ("A".to_string(), Some(11)),
            ("B".to_string(), Some(12)),
        ]
    );
}

#[tokio::test]
async fn reorder_within_unknown_or_repeated_id_writes_nothing() {
    let (_d, db) = fresh_db().await;
    let p = db.pool();
    let a = item(p, "A", Some(0)).await;
    let b = item(p, "B", Some(1)).await;
    let c = item(p, "C", None).await;
    let before = global(p).await;

    // The unknown id comes LAST, after real ids that would otherwise have been written.
    let err = Items::reorder_within(p, &[c.clone(), b.clone(), a.clone(), "ai-gone".into()])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no longer exists"), "{err}");
    assert_eq!(global(p).await, before);

    assert!(Items::reorder_within(p, &[b.clone(), a.clone(), b.clone()])
        .await
        .is_err());
    assert_eq!(global(p).await, before);

    assert_eq!(Items::reorder_within(p, &[]).await.unwrap(), 0);
}

// ---- hide_from_todo filter ------------------------------------------------------------

async fn ids(pool: &SqlitePool, v: TodoView) -> Vec<String> {
    Todo::list_view(pool, v)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect()
}

#[tokio::test]
async fn hidden_topic_leaves_the_list_view_only() {
    let (_d, db) = fresh_db().await;
    let p = db.pool();
    let hidden = Topics::create(p, "Side project", "").await.unwrap();
    let shown = Topics::create(p, "Work", "").await.unwrap();
    let h_open = item(p, "h-open", Some(0)).await;
    let plain = item(p, "plain", Some(1)).await;
    let s_open = item(p, "s-open", Some(2)).await;
    let h_done = item(p, "h-done", Some(3)).await;
    for (it, t) in [(&h_open, &hidden), (&h_done, &hidden), (&s_open, &shown)] {
        assert!(Todo::set_topic(p, it, Some(&t.id)).await.unwrap());
    }
    Items::set_status(p, &h_done, "completed").await.unwrap();
    // A suggestion tagged to the hidden topic stays in the Inbox.
    let sugg = item(p, "h-sugg", Some(4)).await;
    sqlx::query("UPDATE action_items SET triage = 'suggested', topic_id = ? WHERE id = ?")
        .bind(&hidden.id)
        .bind(&sugg)
        .execute(p)
        .await
        .unwrap();

    assert_eq!(
        ids(p, TodoView::List).await,
        [h_open.clone(), plain.clone(), s_open.clone()]
    );

    Topics::patch(
        p,
        &hidden.id,
        &TopicPatch {
            hide_from_todo: Some(true),
            ..TopicPatch::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        ids(p, TodoView::List).await,
        [plain.clone(), s_open.clone()]
    );
    assert_eq!(ids(p, TodoView::Done).await, std::slice::from_ref(&h_done));
    assert_eq!(ids(p, TodoView::Inbox).await, std::slice::from_ref(&sugg));
    let card: Vec<String> = Todo::list_for_topic(p, &hidden.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.item.id)
        .collect();
    assert_eq!(card, [h_open.clone(), h_done.clone()]);

    // Turning it off brings them back in their old positions.
    Topics::patch(
        p,
        &hidden.id,
        &TopicPatch {
            hide_from_todo: Some(false),
            ..TopicPatch::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ids(p, TodoView::List).await, [h_open, plain, s_open]);
}
