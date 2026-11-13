//! specs/0089: topic repository, ledger rules, delete cascades.

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::topics::repo::TopicsRepository as Topics;
use common::fresh_db;
use sqlx::SqlitePool;

const NOW: &str = "2026-01-01T00:00:00Z";

async fn count(pool: &SqlitePool, sql: &str, bind: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn create_list_update_close_reopen() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    assert!(Topics::create(p, "   ", "").await.is_err());
    let a = Topics::create(p, "Zeta rollout", "d").await.unwrap();
    let b = Topics::create(p, "Atlas CRM rebuild", "").await.unwrap();
    assert_eq!(a.status, "open");
    let closed = Topics::update(p, &a.id, None, None, Some("closed"))
        .await
        .unwrap()
        .unwrap();
    assert!(closed.closed_at.is_some());
    let open = Topics::list(p, Some("open")).await.unwrap();
    assert_eq!(
        open.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        [b.id.as_str()]
    );
    let all = Topics::list(p, None).await.unwrap();
    assert_eq!(all[0].id, b.id);
    assert_eq!(all[1].id, a.id);
    let reopened = Topics::update(p, &a.id, Some("Zeta"), None, Some("open"))
        .await
        .unwrap()
        .unwrap();
    assert!(reopened.closed_at.is_none());
    assert_eq!(reopened.name, "Zeta");
    assert!(Topics::update(p, "nope", Some("x"), None, None)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn set_notes_round_trips() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    assert!(Topics::set_notes(p, &t.id, "# hi", Some("<h1>hi</h1>"))
        .await
        .unwrap());
    let got = Topics::get(p, &t.id).await.unwrap().unwrap();
    assert_eq!(got.notes_markdown.as_deref(), Some("# hi"));
    assert_eq!(got.notes_json.as_deref(), Some("<h1>hi</h1>"));
    assert!(!Topics::set_notes(p, "nope", "x", None).await.unwrap());
}

async fn seed_children(pool: &SqlitePool, topic: &str, meeting: &str) {
    sqlx::query(
        "INSERT INTO topic_entries (id, topic_id, meeting_id, kind, occurred_at, ai_body_json, body_json, created_at)
         VALUES ('te-1', ?, ?, 'series', ?, '{}', '{}', ?)",
    )
    .bind(topic).bind(meeting).bind(NOW).bind(NOW)
    .execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO topic_revisions (id, entry_id, text, source_meeting_id, created_at)
         VALUES ('tr-1', 'te-1', 'rev', ?, ?)",
    )
    .bind(meeting)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
    for (id, status) in [("ts-1", "pending"), ("ts-2", "dismissed")] {
        sqlx::query(
            "INSERT INTO topic_suggestions (id, topic_id, kind, meeting_id, text, status, created_at)
             VALUES (?, ?, 'note', ?, 'x', ?, ?)",
        )
        .bind(id).bind(topic).bind(meeting).bind(status).bind(NOW)
        .execute(pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO topic_links (topic_id, kind, ref, created_at) VALUES (?, 'meeting', ?, ?)",
    )
    .bind(topic)
    .bind(meeting)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO topic_mention_scans (meeting_id, fingerprint, created_at) VALUES (?, 'f', ?)",
    )
    .bind(meeting)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_meeting_and_todo(pool: &SqlitePool, meeting: &str, topic: &str) {
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, 'M', ?, ?)")
        .bind(meeting)
        .bind(NOW)
        .bind(NOW)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO action_items (id, meeting_id, description, content_key, status, triage, topic_id, created_at, updated_at)
         VALUES ('ai-1', ?, 'do it', 'do it', 'open', 'listed', ?, ?, ?)",
    )
    .bind(meeting).bind(topic).bind(NOW).bind(NOW)
    .execute(pool).await.unwrap();
}

#[tokio::test]
async fn delete_topic_cascades_and_untags_todos() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    seed_meeting_and_todo(p, "m-1", &t.id).await;
    seed_children(p, &t.id, "m-1").await;
    assert!(Topics::delete(p, &t.id).await.unwrap());
    assert!(!Topics::delete(p, &t.id).await.unwrap());
    for table in ["topic_entries", "topic_suggestions", "topic_links"] {
        let n = count(
            p,
            &format!("SELECT COUNT(*) FROM {table} WHERE topic_id = ?"),
            &t.id,
        )
        .await;
        assert_eq!(n, 0, "{table}");
    }
    assert_eq!(
        count(
            p,
            "SELECT COUNT(*) FROM topic_revisions WHERE entry_id = ?",
            "te-1"
        )
        .await,
        0
    );
    let tid: Option<String> =
        sqlx::query_scalar("SELECT topic_id FROM action_items WHERE id = 'ai-1'")
            .fetch_one(p)
            .await
            .unwrap();
    assert!(tid.is_none());
}

#[tokio::test]
async fn meeting_delete_keeps_entries_and_drops_pending() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    seed_meeting_and_todo(p, "m-1", &t.id).await;
    seed_children(p, &t.id, "m-1").await;
    MeetingsRepository::delete_meeting(p, "m-1").await.unwrap();
    let entry: Option<String> =
        sqlx::query_scalar("SELECT source_deleted_at FROM topic_entries WHERE id = 'te-1'")
            .fetch_one(p)
            .await
            .unwrap();
    assert!(entry.is_some());
    let rev: Option<String> =
        sqlx::query_scalar("SELECT source_deleted_at FROM topic_revisions WHERE id = 'tr-1'")
            .fetch_one(p)
            .await
            .unwrap();
    assert!(rev.is_some());
    let left: Vec<String> = sqlx::query_scalar("SELECT id FROM topic_suggestions ORDER BY id")
        .fetch_all(p)
        .await
        .unwrap();
    assert_eq!(left, ["ts-2"]);
    assert_eq!(
        count(p, "SELECT COUNT(*) FROM topic_links WHERE ref = ?", "m-1").await,
        0
    );
    assert_eq!(
        count(
            p,
            "SELECT COUNT(*) FROM topic_mention_scans WHERE meeting_id = ?",
            "m-1"
        )
        .await,
        0
    );
    let tid: Option<String> =
        sqlx::query_scalar("SELECT topic_id FROM action_items WHERE id = 'ai-1'")
            .fetch_one(p)
            .await
            .unwrap();
    assert_eq!(tid.as_deref(), Some(t.id.as_str()));
}

// ---- Task 2: links, membership, backfill (specs/0089) ----

mod links_tests {
    use super::*;
    use app_lib::topics::links::{self, LinkKind};
    use chrono::{DateTime, Duration, Utc};
    use std::collections::BTreeSet;

    async fn meeting(
        pool: &SqlitePool,
        id: &str,
        title: &str,
        key: Option<&str>,
        at: DateTime<Utc>,
        summary: bool,
    ) {
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
        if summary {
            sqlx::query(
                "INSERT INTO summary_processes (meeting_id, status, created_at, updated_at)
                 VALUES (?, 'completed', ?, ?)",
            )
            .bind(id)
            .bind(NOW)
            .bind(NOW)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn series_links(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM meeting_series_links")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn members(pool: &SqlitePool, topic: &str, ids: &[&str]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for id in ids {
            let ts = links::topics_for_meeting(pool, id, false).await.unwrap();
            if ts.iter().any(|t| t.id == topic) {
                out.insert(id.to_string());
            }
        }
        out
    }

    #[tokio::test]
    async fn membership_matches_find_prior_series_occurrences() {
        let (_d, m) = fresh_db().await;
        let p = m.pool();
        let base = Utc::now() - Duration::days(30);
        let mut all: Vec<String> = Vec::new();
        for (i, id) in ["a1", "a2", "a3"].iter().enumerate() {
            meeting(
                p,
                id,
                "Weekly 1:1",
                Some("cal:a"),
                base + Duration::days(i as i64),
                true,
            )
            .await;
            all.push(id.to_string());
        }
        for (i, id) in ["b1", "b2"].iter().enumerate() {
            meeting(
                p,
                id,
                "weekly 1:1 ",
                Some("cal:b"),
                base + Duration::days(i as i64),
                true,
            )
            .await;
            all.push(id.to_string());
        }
        meeting(p, "u1", " WEEKLY 1:1", None, base + Duration::days(4), true).await;
        all.push("u1".into());
        for (i, id) in ["t1", "t2"].iter().enumerate() {
            meeting(
                p,
                id,
                "Atlas sync",
                None,
                base + Duration::days(i as i64),
                true,
            )
            .await;
            all.push(id.to_string());
        }
        let ids: Vec<&str> = all.iter().map(String::as_str).collect();
        let far = Utc::now() + Duration::days(3650);

        let ta = Topics::create(p, "A", "").await.unwrap();
        let la = links::link_series_of(p, &ta.id, "a3").await.unwrap();
        assert_eq!(la.kind, LinkKind::Series);
        assert_eq!(la.ref_, "cal:a");
        let mut want: BTreeSet<String> = MeetingsRepository::find_prior_series_occurrences(
            p,
            Some("cal:a"),
            "Weekly 1:1",
            far,
            500,
        )
        .await
        .unwrap()
        .into_iter()
        .collect();
        want.insert("a3".into());
        let got = members(p, &ta.id, &ids).await;
        assert_eq!(got, want);
        assert!(got.contains("u1"), "unaffiliated same-title joins");
        assert!(
            !got.contains("b1") && !got.contains("b2"),
            "series B never matches A"
        );

        let tt = Topics::create(p, "Atlas", "").await.unwrap();
        let lt = links::link_series_of(p, &tt.id, "t2").await.unwrap();
        assert_eq!(lt.kind, LinkKind::Title);
        assert_eq!(lt.ref_, "atlas sync");
        let mut want: BTreeSet<String> =
            MeetingsRepository::find_prior_series_occurrences(p, None, "Atlas sync", far, 500)
                .await
                .unwrap()
                .into_iter()
                .collect();
        want.insert("t2".into());
        assert_eq!(members(p, &tt.id, &ids).await, want);
    }

    #[tokio::test]
    async fn link_series_never_writes_series_links() {
        let (_d, m) = fresh_db().await;
        let p = m.pool();
        meeting(p, "a1", "Weekly", Some("cal:a"), Utc::now(), true).await;
        meeting(p, "n1", "  ", None, Utc::now(), true).await;
        let t = Topics::create(p, "A", "").await.unwrap();
        let before = series_links(p).await;
        links::link_series_of(p, &t.id, "a1").await.unwrap();
        links::link_meeting(p, &t.id, "a1").await.unwrap();
        let after = series_links(p).await;
        assert_eq!(before, after);

        let err = links::link_series_of(p, &t.id, "n1").await.unwrap_err();
        assert_eq!(err.to_string(), "This meeting isn't part of a series.");
    }

    #[tokio::test]
    async fn backfill_candidates_oldest_first_within_90_days() {
        let (_d, m) = fresh_db().await;
        let p = m.pool();
        let now = Utc::now();
        meeting(
            p,
            "old",
            "W",
            Some("cal:a"),
            now - Duration::days(100),
            true,
        )
        .await;
        meeting(
            p,
            "nosum",
            "W",
            Some("cal:a"),
            now - Duration::days(10),
            false,
        )
        .await;
        meeting(p, "m3", "W", Some("cal:a"), now - Duration::days(5), true).await;
        meeting(p, "m1", "W", Some("cal:a"), now - Duration::days(60), true).await;
        meeting(p, "m2", "W", Some("cal:a"), now - Duration::days(20), true).await;
        let t = Topics::create(p, "A", "").await.unwrap();
        let l = links::link_series_of(p, &t.id, "m3").await.unwrap();
        let got = links::backfill_candidates(p, &l, now).await.unwrap();
        assert_eq!(got, ["m1", "m2", "m3"]);
    }

    #[tokio::test]
    async fn link_is_idempotent_and_unlink_removes() {
        let (_d, m) = fresh_db().await;
        let p = m.pool();
        meeting(p, "a1", "Weekly", Some("cal:a"), Utc::now(), true).await;
        let t = Topics::create(p, "A", "").await.unwrap();
        links::link_meeting(p, &t.id, "a1").await.unwrap();
        let l = links::link_meeting(p, &t.id, "a1").await.unwrap();
        assert_eq!(l.label, "Weekly");
        assert_eq!(links::links_for_topic(p, &t.id).await.unwrap().len(), 1);
        links::link_series_of(p, &t.id, "a1").await.unwrap();
        links::link_series_of(p, &t.id, "a1").await.unwrap();
        assert_eq!(links::links_for_topic(p, &t.id).await.unwrap().len(), 2);
        assert!(links::unlink(p, &t.id, LinkKind::Meeting, "a1")
            .await
            .unwrap());
        assert!(!links::unlink(p, &t.id, LinkKind::Meeting, "a1")
            .await
            .unwrap());
        assert_eq!(links::links_for_topic(p, &t.id).await.unwrap().len(), 1);
        // open_only excludes closed topics
        assert_eq!(
            links::topics_for_meeting(p, "a1", true)
                .await
                .unwrap()
                .len(),
            1
        );
        Topics::update(p, &t.id, None, None, Some("closed"))
            .await
            .unwrap();
        assert!(links::topics_for_meeting(p, "a1", true)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            links::topics_for_meeting(p, "a1", false)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
