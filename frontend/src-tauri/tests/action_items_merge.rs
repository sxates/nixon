//! specs/0088 W2 (Task 6): cross-meeting source persistence and the regeneration-safe
//! detach/repoint. Drives `replace_extracted` with hand-built diffs; no LLM.

mod common;

use app_lib::action_items::diff::{
    content_key, CandidateInsert, ExtractionDiff, PristineUpdate, ResolvedCandidate,
};
use app_lib::action_items::merge::SourceAttach;
use app_lib::database::repositories::action_item::ActionItemsRepository as Repo;
use chrono::{Duration, Utc};
use common::fresh_db;
use sqlx::SqlitePool;

fn cand(desc: &str) -> ResolvedCandidate {
    ResolvedCandidate {
        description: desc.to_string(),
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

async fn extracted(pool: &SqlitePool, meeting_id: &str, desc: &str) -> String {
    Repo::create(pool, Some(meeting_id), &cand(desc), "extracted", desc)
        .await
        .unwrap()
        .id
}

fn attach(item: &str, ctx: &str) -> SourceAttach {
    SourceAttach {
        item_id: item.to_string(),
        context: Some(ctx.to_string()),
        phrasing: format!("phrasing {ctx}"),
    }
}

async fn run(pool: &SqlitePool, m: &str, diff: &ExtractionDiff, attaches: &[SourceAttach]) {
    let out = Repo::replace_extracted(pool, m, diff, attaches, "fp", "p", "m", 1)
        .await
        .unwrap();
    assert!(out.is_some(), "meeting exists");
}

fn delete_diff(ids: &[&str]) -> ExtractionDiff {
    ExtractionDiff {
        delete_ids: ids.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

/// (meeting_id, context) pairs of an item's source rows, sorted.
async fn sources(pool: &SqlitePool, item: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT meeting_id, context FROM action_item_sources WHERE action_item_id = ?
         ORDER BY meeting_id",
    )
    .bind(item)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn count_items(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM action_items")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn attach_adds_second_source_no_new_item() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "b", 1).await;
    let x = extracted(pool, "a", "Send the budget").await;
    run(
        pool,
        "b",
        &ExtractionDiff::default(),
        &[attach(&x, "again")],
    )
    .await;
    assert_eq!(count_items(pool).await, 1);
    let item = Repo::get(pool, &x).await.unwrap().unwrap();
    assert_eq!(item.meeting_id.as_deref(), Some("a"), "still born in a");
    let s = sources(pool, &x).await;
    assert_eq!(s.len(), 2);
    assert_eq!(s[1], ("b".to_string(), Some("again".to_string())));
}

#[tokio::test]
async fn reextract_replaces_only_this_meetings_attaches() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "b", 1).await;
    let x = extracted(pool, "a", "Send the budget").await;
    let y = extracted(pool, "a", "Book the venue").await;
    run(
        pool,
        "b",
        &ExtractionDiff::default(),
        &[attach(&x, "x1"), attach(&y, "y1")],
    )
    .await;
    run(pool, "b", &ExtractionDiff::default(), &[attach(&y, "y2")]).await;
    assert_eq!(sources(pool, &x).await.len(), 1, "stale b attach on x gone");
    let sy = sources(pool, &y).await;
    assert_eq!(sy.len(), 2);
    assert_eq!(
        sy[1].1.as_deref(),
        Some("y2"),
        "attach replaced, not duplicated"
    );
    assert_eq!(
        sources(pool, &x).await[0].0,
        "a",
        "a's own source untouched"
    );
}

#[tokio::test]
async fn reextract_skips_attach_to_deleted_item() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "b", 1).await;
    let x = extracted(pool, "a", "Send the budget").await;
    assert!(Repo::delete(pool, &x).await.unwrap());
    // `Repo::delete` leaves the primary source row behind; clear it so only the stale
    // attach could create an orphan.
    sqlx::query("DELETE FROM action_item_sources WHERE action_item_id = ?")
        .bind(&x)
        .execute(pool)
        .await
        .unwrap();
    run(
        pool,
        "b",
        &ExtractionDiff::default(),
        &[attach(&x, "stale")],
    )
    .await;
    let orphans: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM action_item_sources WHERE action_item_id = ?")
            .bind(&x)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(orphans, 0, "no orphan source row");
}

#[tokio::test]
async fn repoint_conflict_leaves_item_intact() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "m", 1).await;
    let x = extracted(pool, "m", "Send the budget").await;
    // A different row in `a` already holds the same content_key: repointing x there
    // would violate the unique index.
    let _y = extracted(pool, "a", "Send the budget").await;
    run(
        pool,
        "a",
        &ExtractionDiff::default(),
        &[attach(&x, "from a")],
    )
    .await;
    run(pool, "m", &delete_diff(&[&x]), &[]).await;
    let item = Repo::get(pool, &x).await.unwrap().expect("not deleted");
    assert_eq!(item.meeting_id.as_deref(), Some("m"));
    let s = sources(pool, &x).await;
    assert_eq!(
        s.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["a", "m"],
        "m's source row kept"
    );
}

#[tokio::test]
async fn attach_to_standalone_todo() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m", 1).await;
    let t = Repo::create(pool, None, &cand("Renew the licence"), "manual", "x")
        .await
        .unwrap()
        .id;
    run(pool, "m", &ExtractionDiff::default(), &[attach(&t, "said")]).await;
    let item = Repo::get(pool, &t).await.unwrap().unwrap();
    assert_eq!(item.meeting_id, None, "stays standalone");
    assert_eq!(sources(pool, &t).await.len(), 1);
    run(pool, "m", &ExtractionDiff::default(), &[]).await;
    assert!(sources(pool, &t).await.is_empty(), "m's attach replaced");
    assert!(Repo::get(pool, &t).await.unwrap().is_some());
}

#[tokio::test]
async fn pristine_multi_source_item_survives_when_m_drops_it() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "c", 3).await;
    meeting(pool, "m", 1).await;
    let x = extracted(pool, "m", "Send the budget").await;
    run(pool, "c", &ExtractionDiff::default(), &[attach(&x, "c")]).await;
    run(pool, "a", &ExtractionDiff::default(), &[attach(&x, "a")]).await;
    run(pool, "m", &delete_diff(&[&x]), &[]).await;
    let item = Repo::get(pool, &x).await.unwrap().expect("survives");
    assert_eq!(item.meeting_id.as_deref(), Some("a"), "oldest other source");
    let s = sources(pool, &x).await;
    assert_eq!(
        s.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        vec!["a", "c"]
    );
}

#[tokio::test]
async fn single_source_pristine_item_still_deleted_with_its_rows() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m", 1).await;
    let x = extracted(pool, "m", "Send the budget").await;
    sqlx::query("INSERT INTO people (id, display_name, created_at, updated_at) VALUES ('p-1','Pat','2026-01-01','2026-01-01')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO action_item_people (action_item_id, person_id) VALUES (?, 'p-1')")
        .bind(&x)
        .execute(pool)
        .await
        .unwrap();
    run(pool, "m", &delete_diff(&[&x]), &[]).await;
    assert!(Repo::get(pool, &x).await.unwrap().is_none());
    assert!(sources(pool, &x).await.is_empty());
    let people: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM action_item_people")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(people, 0);
}

#[tokio::test]
async fn attach_to_dismissed_stays_dismissed() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "b", 1).await;
    let x = extracted(pool, "a", "Send the budget").await;
    sqlx::query("UPDATE action_items SET status = 'dismissed', user_edited = 1 WHERE id = ?")
        .bind(&x)
        .execute(pool)
        .await
        .unwrap();
    let before = Repo::get(pool, &x).await.unwrap().unwrap();
    run(
        pool,
        "b",
        &ExtractionDiff::default(),
        &[attach(&x, "again")],
    )
    .await;
    let after = Repo::get(pool, &x).await.unwrap().unwrap();
    assert_eq!(after.status, "dismissed");
    assert_eq!(after.triage, before.triage);
    assert_eq!(after.description, before.description);
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(sources(pool, &x).await.len(), 2);
}

#[tokio::test]
async fn attach_to_recently_completed_stays_completed() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "a", 5).await;
    meeting(pool, "b", 1).await;
    let x = extracted(pool, "a", "Send the budget").await;
    sqlx::query(
        "UPDATE action_items SET status = 'completed', triage = 'listed', user_edited = 1,
         completed_at = ? WHERE id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(&x)
    .execute(pool)
    .await
    .unwrap();
    let before = Repo::get(pool, &x).await.unwrap().unwrap();
    run(
        pool,
        "b",
        &ExtractionDiff::default(),
        &[attach(&x, "again")],
    )
    .await;
    let after = Repo::get(pool, &x).await.unwrap().unwrap();
    assert_eq!(after.status, "completed");
    assert_eq!(after.triage, "listed");
    assert_eq!(after.completed_at, before.completed_at);
    assert_eq!(sources(pool, &x).await.len(), 2);
}

#[tokio::test]
async fn listed_items_never_modified_by_reextraction() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m", 1).await;
    let x = extracted(pool, "m", "Send the budget").await;
    sqlx::query("UPDATE action_items SET triage = 'listed', user_edited = 1 WHERE id = ?")
        .bind(&x)
        .execute(pool)
        .await
        .unwrap();
    let before = Repo::get(pool, &x).await.unwrap().unwrap();
    // A stale plan that (wrongly) targets the row: the pristine guards must hold.
    let diff = ExtractionDiff {
        updates: vec![PristineUpdate {
            id: x.clone(),
            candidate: ResolvedCandidate {
                context: Some("new".to_string()),
                ..cand("Totally different words")
            },
            content_key: content_key("Totally different words"),
        }],
        delete_ids: vec![x.clone()],
        ..Default::default()
    };
    run(pool, "m", &diff, &[]).await;
    let after = Repo::get(pool, &x).await.unwrap().expect("still there");
    assert_eq!(after.description, before.description);
    assert_eq!(after.triage, "listed");
    assert_eq!(after.updated_at, before.updated_at);
    let s = sources(pool, &x).await;
    assert_eq!(s, vec![("m".to_string(), None)], "primary row untouched");
}

#[tokio::test]
async fn inserts_and_pristine_updates_upsert_primary_source() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    meeting(pool, "m", 1).await;
    let with_ctx = |d: &str, c: &str| ResolvedCandidate {
        context: Some(c.to_string()),
        ..cand(d)
    };
    let diff = ExtractionDiff {
        inserts: vec![CandidateInsert {
            candidate: with_ctx("Send the budget", "first"),
            content_key: content_key("Send the budget"),
        }],
        ..Default::default()
    };
    run(pool, "m", &diff, &[]).await;
    let id: String = sqlx::query_scalar("SELECT id FROM action_items")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        sources(pool, &id).await,
        vec![("m".into(), Some("first".into()))]
    );

    let diff = ExtractionDiff {
        updates: vec![PristineUpdate {
            id: id.clone(),
            candidate: with_ctx("Send the budget today", "second"),
            content_key: content_key("Send the budget today"),
        }],
        ..Default::default()
    };
    run(pool, "m", &diff, &[]).await;
    let s: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT meeting_id, context, phrasing FROM action_item_sources WHERE action_item_id = ?",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        s,
        vec![(
            "m".into(),
            Some("second".into()),
            Some("Send the budget today".into())
        )]
    );
}
