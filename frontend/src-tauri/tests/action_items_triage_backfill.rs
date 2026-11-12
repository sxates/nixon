//! specs/0088 W1: the triage migration backfills existing action items correctly.
//! Brings a database up to just before the migration, inserts one row per case, then runs
//! the full migrator.

use std::borrow::Cow;

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;

const V: i64 = 20261112000000;

#[allow(clippy::too_many_arguments)]
async fn insert(
    pool: &sqlx::SqlitePool,
    id: &str,
    meeting: Option<&str>,
    source: &str,
    is_self: i64,
    person: Option<&str>,
    raw: Option<&str>,
    status: &str,
    due_date: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO action_items
            (id, meeting_id, description, assignee_person_id, assignee_is_self, assignee_raw,
             due_date, status, source, user_edited, content_key, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z')",
    )
    .bind(id)
    .bind(meeting)
    .bind(format!("desc {id}"))
    .bind(person)
    .bind(is_self)
    .bind(raw)
    .bind(due_date)
    .bind(status)
    .bind(source)
    .bind(format!("key-{id}"))
    .execute(pool)
    .await
    .expect("insert row");
}

#[tokio::test]
async fn migration_backfills_triage_and_sources() {
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

    insert(&pool, "manual", None, "manual", 1, None, None, "open", None).await;
    // Legacy listed rows: one with an explicit order, one without (NULL sorts last).
    insert(
        &pool, "ordered", None, "manual", 1, None, None, "open", None,
    )
    .await;
    sqlx::query("UPDATE action_items SET sort_order = 5 WHERE id = 'ordered'")
        .execute(&pool)
        .await
        .unwrap();
    insert(
        &pool,
        "done",
        Some("m1"),
        "extracted",
        1,
        None,
        None,
        "completed",
        None,
    )
    .await;
    insert(
        &pool,
        "dated",
        Some("m1"),
        "extracted",
        1,
        None,
        None,
        "open",
        Some("2026-02-01"),
    )
    .await;
    insert(
        &pool,
        "pristine",
        Some("m1"),
        "extracted",
        1,
        None,
        None,
        "open",
        None,
    )
    .await;
    insert(
        &pool,
        "nobody",
        Some("m2"),
        "extracted",
        0,
        None,
        None,
        "open",
        None,
    )
    .await;
    insert(
        &pool,
        "raw",
        Some("m2"),
        "extracted",
        0,
        None,
        Some("Sam"),
        "open",
        None,
    )
    .await;
    insert(
        &pool,
        "person",
        Some("m2"),
        "extracted",
        0,
        Some("p-1"),
        None,
        "open",
        None,
    )
    .await;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("full migrations");

    let triage = |id: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query("SELECT triage FROM action_items WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<String, _>(0)
        }
    };
    assert_eq!(triage("manual").await, "listed");
    assert_eq!(triage("done").await, "listed");
    assert_eq!(triage("dated").await, "listed");
    assert_eq!(triage("pristine").await, "suggested");
    assert_eq!(triage("nobody").await, "suggested");
    assert_eq!(triage("raw").await, "record");
    assert_eq!(triage("person").await, "record");

    // Listed rows are numbered densely 0..n: explicit order first, then NULL by created_at.
    let order: Vec<(String, Option<i64>)> = sqlx::query_as(
        "SELECT id, sort_order FROM action_items WHERE triage = 'listed' ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let sorted: Vec<i64> = order.iter().filter_map(|(_, s)| *s).collect();
    assert_eq!(sorted, (0..order.len() as i64).collect::<Vec<_>>());
    assert_eq!(order[0].0, "ordered");
    let item =
        app_lib::database::repositories::action_item_todo::ActionItemTodoRepository::add_to_list(
            &pool, "pristine",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(item.sort_order, Some(order.len() as i64));

    // Listed rows carry adopted_at; others do not.
    let adopted: Option<String> =
        sqlx::query_scalar("SELECT adopted_at FROM action_items WHERE id = 'done'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(adopted.as_deref(), Some("2026-01-02T00:00:00Z"));
    let adopted: Option<String> =
        sqlx::query_scalar("SELECT adopted_at FROM action_items WHERE id = 'raw'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(adopted.is_none());

    // Exactly one source row per item with a meeting, phrasing == description; none otherwise.
    let rows = sqlx::query(
        "SELECT ai.id, ai.description, COUNT(s.meeting_id) AS n, MIN(s.phrasing) AS p
         FROM action_items ai LEFT JOIN action_item_sources s ON s.action_item_id = ai.id
         GROUP BY ai.id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 8);
    for r in rows {
        let id: String = r.get("id");
        let n: i64 = r.get("n");
        if id == "manual" || id == "ordered" {
            assert_eq!(n, 0);
        } else {
            assert_eq!(n, 1, "{id}");
            assert_eq!(r.get::<String, _>("p"), r.get::<String, _>("description"));
        }
    }
}

// ---- create / replace_extracted write the triage columns (specs/0088 W1) ----

mod common;

use app_lib::action_items::diff::{
    self, CandidateInsert, ExtractionDiff, PristineUpdate, ResolvedCandidate,
};
use app_lib::database::repositories::action_item::ActionItemsRepository;
use app_lib::database::repositories::meeting::MeetingsRepository;
use common::fresh_db;

fn cand(description: &str, is_self: bool, person: Option<&str>) -> ResolvedCandidate {
    ResolvedCandidate {
        description: description.to_string(),
        assignee_person_id: person.map(String::from),
        assignee_is_self: is_self,
        assignee_raw: None,
        due_hint: None,
        due_date: None,
        ..Default::default()
    }
}

#[tokio::test]
async fn create_manual_is_listed_and_appends() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    for (i, d) in ["one", "two"].iter().enumerate() {
        let it = ActionItemsRepository::create(
            pool,
            None,
            &cand(d, true, None),
            "manual",
            &diff::content_key(d),
        )
        .await
        .unwrap();
        assert_eq!(it.sort_order, Some(i as i64));
    }
    let third = ActionItemsRepository::create(
        pool,
        None,
        &cand("three", true, None),
        "manual",
        &diff::content_key("three"),
    )
    .await
    .unwrap();
    assert_eq!(third.triage, "listed");
    assert_eq!(third.sort_order, Some(2));
    assert!(third.adopted_at.is_some());
}

#[tokio::test]
async fn create_extracted_uses_triage_for() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let mine =
        ActionItemsRepository::create(pool, None, &cand("mine", true, None), "extracted", "k1")
            .await
            .unwrap();
    let theirs = ActionItemsRepository::create(
        pool,
        None,
        &cand("theirs", false, Some("p-1")),
        "extracted",
        "k2",
    )
    .await
    .unwrap();
    assert_eq!(mine.triage, "suggested");
    assert_eq!(theirs.triage, "record");
    for it in [&mine, &theirs] {
        assert!(it.adopted_at.is_none());
        assert!(it.sort_order.is_none());
    }
}

#[tokio::test]
async fn create_with_meeting_writes_source_row() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let m = MeetingsRepository::create_meeting(pool, Some("Sync".into()), None, None, None, None)
        .await
        .unwrap();
    let it = ActionItemsRepository::create(
        pool,
        Some(&m),
        &cand("ship it", true, None),
        "extracted",
        "k",
    )
    .await
    .unwrap();
    let row = sqlx::query(
        "SELECT meeting_id, context, phrasing FROM action_item_sources WHERE action_item_id = ?",
    )
    .bind(&it.id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].get::<String, _>("meeting_id"), m);
    assert!(row[0].get::<Option<String>, _>("context").is_none());
    assert_eq!(row[0].get::<String, _>("phrasing"), "ship it");
}

#[tokio::test]
async fn replace_extracted_sets_triage_on_insert_and_pristine_update() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let m = MeetingsRepository::create_meeting(pool, Some("Sync".into()), None, None, None, None)
        .await
        .unwrap();
    let first = ExtractionDiff {
        inserts: vec![CandidateInsert {
            candidate: cand("draft plan", false, None),
            content_key: diff::content_key("draft plan"),
        }],
        ..Default::default()
    };
    ActionItemsRepository::replace_extracted(pool, &m, &first, &[], "fp1", "p", "m", 1)
        .await
        .unwrap();
    let rows = ActionItemsRepository::get_for_meeting(pool, &m)
        .await
        .unwrap();
    assert_eq!(rows[0].triage, "suggested");

    let second = ExtractionDiff {
        updates: vec![PristineUpdate {
            id: rows[0].id.clone(),
            candidate: cand("draft plan", false, Some("p-1")),
            content_key: rows[0].content_key.clone(),
        }],
        ..Default::default()
    };
    ActionItemsRepository::replace_extracted(pool, &m, &second, &[], "fp2", "p", "m", 1)
        .await
        .unwrap();
    let rows = ActionItemsRepository::get_for_meeting(pool, &m)
        .await
        .unwrap();
    assert_eq!(rows[0].triage, "record");
}
