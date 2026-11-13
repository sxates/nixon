//! specs/0089: topic repository, ledger rules, delete cascades.

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::topics::ledger::{self, NewSuggestion};
use app_lib::topics::model::{Bullet, Decision, EntryBody};
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

// ---- ledger (task 3) ----

fn bullet(t: &str) -> Bullet {
    Bullet {
        text: t.into(),
        t: Some(12.0),
    }
}

fn body(bullets: &[&str], decisions: &[(&str, &str)]) -> EntryBody {
    EntryBody {
        bullets: bullets.iter().map(|b| bullet(b)).collect(),
        decisions: decisions
            .iter()
            .map(|(id, t)| Decision {
                id: (*id).into(),
                text: (*t).into(),
                t: None,
            })
            .collect(),
    }
}

async fn meeting(pool: &SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, 'Weekly sync', ?, ?)",
    )
    .bind(id)
    .bind(NOW)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

async fn body_raw(pool: &SqlitePool, id: &str) -> String {
    sqlx::query_scalar("SELECT body_json FROM topic_entries WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn sug(topic: &str, kind: &str, meeting: &str, text: &str) -> NewSuggestion {
    NewSuggestion {
        topic_id: topic.into(),
        kind: kind.into(),
        meeting_id: meeting.into(),
        t_start: Some(30.0),
        target_entry_id: None,
        target_decision_id: None,
        text: text.into(),
    }
}

#[tokio::test]
async fn insert_entry_refuses_closed_topic_or_deleted_meeting() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    assert!(
        ledger::insert_machine_entry(p, &t.id, "gone", "series", body(&["a"], &[]))
            .await
            .unwrap()
            .is_none()
    );
    Topics::update(p, &t.id, None, None, Some("closed"))
        .await
        .unwrap();
    assert!(
        ledger::insert_machine_entry(p, &t.id, "m-1", "series", body(&["a"], &[]))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        ledger::insert_machine_entry(p, "nope", "m-1", "series", body(&["a"], &[]))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        count(
            p,
            "SELECT COUNT(*) FROM topic_entries WHERE topic_id = ?",
            &t.id
        )
        .await,
        0
    );
    // suggestions respect closed topics too
    assert_eq!(
        ledger::insert_suggestions(p, &[sug(&t.id, "note", "m-1", "x")])
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn insert_entry_is_idempotent_per_meeting() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    let id = ledger::insert_machine_entry(p, &t.id, "m-1", "series", body(&["first"], &[]))
        .await
        .unwrap()
        .unwrap();
    let before = body_raw(p, &id).await;
    assert!(
        ledger::insert_machine_entry(p, &t.id, "m-1", "series", body(&["second"], &[]))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(body_raw(p, &id).await, before);
    let e = &ledger::entries_for_topic(p, &t.id).await.unwrap()[0];
    assert_eq!(e.occurred_at, NOW);
    assert_eq!(e.meeting_title.as_deref(), Some("Weekly sync"));
    assert_eq!(e.ai_body.as_ref(), Some(&e.body));
    assert!(ledger::has_entry(p, &t.id, "m-1").await.unwrap());
}

#[tokio::test]
async fn machine_supplied_decision_ids_are_replaced() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    ledger::insert_machine_entry(p, &t.id, "m-1", "series", body(&[], &[("evil", "Ship it")]))
        .await
        .unwrap()
        .unwrap();
    let e = &ledger::entries_for_topic(p, &t.id).await.unwrap()[0];
    assert!(e.body.decisions[0].id.starts_with("dec-"));
    assert_ne!(e.body.decisions[0].id, "evil");
}

#[tokio::test]
async fn edit_marks_edited_and_revert_restores() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    let id =
        ledger::insert_machine_entry(p, &t.id, "m-1", "series", body(&["orig"], &[("x", "D1")]))
            .await
            .unwrap()
            .unwrap();
    let orig = ledger::entries_for_topic(p, &t.id).await.unwrap()[0].clone();
    let did = orig.body.decisions[0].id.clone();
    let mut edited = body(&["mine"], &[]);
    edited.decisions = vec![
        Decision {
            id: did.clone(),
            text: "D1 fixed".into(),
            t: None,
        },
        Decision {
            id: "forged".into(),
            text: "New".into(),
            t: None,
        },
    ];
    assert!(ledger::edit_entry(p, &id, edited).await.unwrap());
    let e = ledger::entries_for_topic(p, &t.id).await.unwrap().remove(0);
    assert!(e.edited_at.is_some());
    assert_eq!(e.body.decisions[0].id, did);
    assert!(e.body.decisions[1].id.starts_with("dec-"));
    assert_eq!(e.ai_body, orig.ai_body);
    assert!(!ledger::edit_entry(p, "nope", body(&[], &[])).await.unwrap());
    assert!(ledger::revert_entry(p, &id).await.unwrap());
    let e = ledger::entries_for_topic(p, &t.id).await.unwrap().remove(0);
    assert_eq!(e.body, orig.body);
    assert!(e.edited_at.is_none());
    // manual entries have nothing to revert to
    let man = ledger::add_manual_entry(p, &t.id, body(&["note"], &[]))
        .await
        .unwrap();
    assert!(!ledger::revert_entry(p, &man).await.unwrap());
    assert!(!ledger::rewrite_entry(p, &man, body(&["x"], &[]))
        .await
        .unwrap());
    let me = ledger::entries_for_topic(p, &t.id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.id == man)
        .unwrap();
    assert_eq!(
        (me.kind.as_str(), me.meeting_id, me.ai_body),
        ("manual", None, None)
    );
}

#[tokio::test]
async fn rewrite_refused_once_edited() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    let id = ledger::insert_machine_entry(p, &t.id, "m-1", "series", body(&["orig"], &[]))
        .await
        .unwrap()
        .unwrap();
    assert!(ledger::rewrite_entry(p, &id, body(&["regen"], &[]))
        .await
        .unwrap());
    ledger::edit_entry(p, &id, body(&["mine"], &[]))
        .await
        .unwrap();
    let before = body_raw(p, &id).await;
    assert!(!ledger::rewrite_entry(p, &id, body(&["regen2"], &[]))
        .await
        .unwrap());
    assert_eq!(body_raw(p, &id).await, before);
    ledger::revert_entry(p, &id).await.unwrap();
    let e = &ledger::entries_for_topic(p, &t.id).await.unwrap()[0];
    assert_eq!(e.body.bullets[0].text, "regen");
}

#[tokio::test]
async fn accept_note_creates_mention_then_merges_second() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-2").await;
    let n = ledger::insert_suggestions(
        p,
        &[
            sug(&t.id, "note", "m-2", "first"),
            sug(&t.id, "note", "m-2", "second"),
        ],
    )
    .await
    .unwrap();
    assert_eq!(n, 2);
    let pend = ledger::pending_suggestions(p, &t.id).await.unwrap();
    assert_eq!(pend[0].meeting_title.as_deref(), Some("Weekly sync"));
    let o1 = ledger::resolve_suggestion(p, &pend[0].id, true)
        .await
        .unwrap();
    let eid = o1.entry_id.unwrap();
    let first_raw = ledger::entries_for_topic(p, &t.id).await.unwrap()[0]
        .body
        .bullets[0]
        .clone();
    let o2 = ledger::resolve_suggestion(p, &pend[1].id, true)
        .await
        .unwrap();
    assert_eq!(o2.entry_id.as_deref(), Some(eid.as_str()));
    let es = ledger::entries_for_topic(p, &t.id).await.unwrap();
    assert_eq!(es.len(), 1);
    assert_eq!(es[0].kind, "mention");
    assert_eq!(es[0].body.bullets.len(), 2);
    assert_eq!(es[0].body.bullets[0], first_raw);
    assert_eq!(es[0].ai_body.as_ref().unwrap().bullets.len(), 2);
    assert!(ledger::pending_suggestions(p, &t.id)
        .await
        .unwrap()
        .is_empty());
}

async fn series_entry_with_revision(pool: &SqlitePool, topic: &str) -> (String, String, String) {
    meeting(pool, "m-1").await;
    meeting(pool, "m-2").await;
    let eid = ledger::insert_machine_entry(
        pool,
        topic,
        "m-1",
        "series",
        body(&["a"], &[("x", "Use Postgres")]),
    )
    .await
    .unwrap()
    .unwrap();
    let did = ledger::entries_for_topic(pool, topic).await.unwrap()[0]
        .body
        .decisions[0]
        .id
        .clone();
    let mut s = sug(topic, "revision", "m-2", "Maybe MySQL");
    s.target_entry_id = Some(eid.clone());
    s.target_decision_id = Some(did.clone());
    ledger::insert_suggestions(pool, &[s]).await.unwrap();
    let sid = ledger::pending_suggestions(pool, topic).await.unwrap()[0]
        .id
        .clone();
    (eid, did, sid)
}

#[tokio::test]
async fn accept_revision_attaches_without_touching_body() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    let (eid, did, sid) = series_entry_with_revision(p, &t.id).await;
    let pend = &ledger::pending_suggestions(p, &t.id).await.unwrap()[0];
    assert_eq!(pend.target_decision_text.as_deref(), Some("Use Postgres"));
    assert_eq!(pend.target_occurred_at.as_deref(), Some(NOW));
    let before = body_raw(p, &eid).await;
    let out = ledger::resolve_suggestion(p, &sid, true).await.unwrap();
    assert_eq!(out.entry_id.as_deref(), Some(eid.as_str()));
    assert_eq!(body_raw(p, &eid).await, before);
    let e = &ledger::entries_for_topic(p, &t.id).await.unwrap()[0];
    assert_eq!(e.revisions.len(), 1);
    assert_eq!(e.revisions[0].decision_id.as_deref(), Some(did.as_str()));
    assert_eq!(e.revisions[0].text, "Maybe MySQL");
    assert_eq!(e.revisions[0].source_meeting_id.as_deref(), Some("m-2"));
    assert!(e.edited_at.is_none());
}

#[tokio::test]
async fn accept_revision_after_target_deleted() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    let (eid, _did, sid) = series_entry_with_revision(p, &t.id).await;
    ledger::delete_entry(p, &eid).await.unwrap();
    let err = ledger::resolve_suggestion(p, &sid, true).await.unwrap_err();
    assert_eq!(err.to_string(), "That entry was deleted.");
    let st: String = sqlx::query_scalar("SELECT status FROM topic_suggestions WHERE id = ?")
        .bind(&sid)
        .fetch_one(p)
        .await
        .unwrap();
    assert_eq!(st, "dismissed");
    assert_eq!(
        count(
            p,
            "SELECT COUNT(*) FROM topic_revisions WHERE entry_id = ?",
            &eid
        )
        .await,
        0
    );
}

#[tokio::test]
async fn accept_revision_after_decision_edited_out() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    let (eid, did, sid) = series_entry_with_revision(p, &t.id).await;
    ledger::edit_entry(p, &eid, body(&["a"], &[]))
        .await
        .unwrap();
    ledger::resolve_suggestion(p, &sid, true).await.unwrap();
    let e = &ledger::entries_for_topic(p, &t.id).await.unwrap()[0];
    assert_eq!(e.revisions[0].decision_id.as_deref(), Some(did.as_str()));
    assert!(e.body.decisions.is_empty());
}

#[tokio::test]
async fn note_suggestion_skipped_when_any_prior_suggestion_or_entry_exists() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    meeting(p, "m-2").await;
    assert_eq!(
        ledger::insert_suggestions(p, &[sug(&t.id, "note", "m-1", "a")])
            .await
            .unwrap(),
        1
    );
    let sid = ledger::pending_suggestions(p, &t.id).await.unwrap()[0]
        .id
        .clone();
    ledger::resolve_suggestion(p, &sid, false).await.unwrap();
    assert_eq!(
        ledger::insert_suggestions(p, &[sug(&t.id, "note", "m-1", "again")])
            .await
            .unwrap(),
        0
    );
    ledger::insert_machine_entry(p, &t.id, "m-2", "series", body(&["x"], &[]))
        .await
        .unwrap();
    assert_eq!(
        ledger::insert_suggestions(p, &[sug(&t.id, "note", "m-2", "b")])
            .await
            .unwrap(),
        0
    );
    let long = "é".repeat(400);
    meeting(p, "m-3").await;
    ledger::insert_suggestions(p, &[sug(&t.id, "note", "m-3", &long)])
        .await
        .unwrap();
    assert_eq!(
        ledger::pending_suggestions(p, &t.id).await.unwrap()[0]
            .text
            .chars()
            .count(),
        280
    );
}

#[tokio::test]
async fn resolve_twice_errors() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    ledger::insert_suggestions(p, &[sug(&t.id, "note", "m-1", "a")])
        .await
        .unwrap();
    let sid = ledger::pending_suggestions(p, &t.id).await.unwrap()[0]
        .id
        .clone();
    ledger::resolve_suggestion(p, &sid, true).await.unwrap();
    for accept in [true, false] {
        let e = ledger::resolve_suggestion(p, &sid, accept)
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "That suggestion was already handled.");
    }
    assert_eq!(ledger::entries_for_topic(p, &t.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn open_decisions_read_owner_text_newest_first_capped() {
    let (_d, m) = fresh_db().await;
    let p = m.pool();
    let t = Topics::create(p, "Atlas", "").await.unwrap();
    meeting(p, "m-1").await;
    let e1 = ledger::insert_machine_entry(
        p,
        &t.id,
        "m-1",
        "series",
        body(&[], &[("x", "Old text"), ("y", "Second")]),
    )
    .await
    .unwrap()
    .unwrap();
    let cur = ledger::entries_for_topic(p, &t.id).await.unwrap()[0]
        .body
        .clone();
    let mut fixed = cur.clone();
    fixed.decisions[0].text = "Owner text".into();
    ledger::edit_entry(p, &e1, fixed).await.unwrap();
    let all = ledger::open_decisions(p, &t.id, 30).await.unwrap();
    assert_eq!(all[0].text, "Owner text");
    assert_eq!(ledger::open_decisions(p, &t.id, 1).await.unwrap().len(), 1);
    assert!(ledger::delete_entry(p, &e1).await.unwrap());
    assert!(ledger::open_decisions(p, &t.id, 30)
        .await
        .unwrap()
        .is_empty());
}
