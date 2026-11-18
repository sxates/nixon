//! Seed helpers and the shared three-meeting fixture for `agent_access_queries.rs`
//! (specs/0087). Raw SQL with fixed ids, dates and audio times so expected links are exact.

#![allow(dead_code)]

use sqlx::SqlitePool;

pub const NOW: &str = "2026-11-08T12:00:00Z";

pub async fn meeting(pool: &SqlitePool, id: &str, title: &str, created_at: &str) {
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(title)
        .bind(created_at)
        .bind(NOW)
        .execute(pool)
        .await
        .unwrap();
}

pub async fn line(
    pool: &SqlitePool,
    meeting_id: &str,
    id: &str,
    text: &str,
    start: Option<f64>,
    speaker: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time,
                                  audio_end_time, duration, speaker)
         VALUES (?, ?, ?, ?, ?, ?, 2.0, ?)",
    )
    .bind(id)
    .bind(meeting_id)
    .bind(text)
    .bind(NOW)
    .bind(start)
    .bind(start.map(|s| s + 2.0))
    .bind(speaker)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn speaker(pool: &SqlitePool, meeting_id: &str, key: &str, name: &str) {
    sqlx::query(
        "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, is_local,
                               created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(format!("speaker-{meeting_id}-{key}"))
    .bind(meeting_id)
    .bind(key)
    .bind(name)
    .bind(i64::from(key == "local"))
    .bind(NOW)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn participant(
    pool: &SqlitePool,
    meeting_id: &str,
    person_id: &str,
    name: &str,
    email: Option<&str>,
) {
    sqlx::query(
        "INSERT OR IGNORE INTO people (id, email, display_name, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(person_id)
    .bind(email)
    .bind(name)
    .bind(NOW)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO meeting_participants (meeting_id, person_id, source, created_at)
         VALUES (?, ?, 'manual', ?)",
    )
    .bind(meeting_id)
    .bind(person_id)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn summary(pool: &SqlitePool, meeting_id: &str, markdown: &str) {
    sqlx::query(
        "INSERT INTO summary_processes (meeting_id, status, created_at, updated_at, result)
         VALUES (?, 'completed', ?, ?, ?)",
    )
    .bind(meeting_id)
    .bind(NOW)
    .bind(NOW)
    .bind(serde_json::json!({ "markdown": markdown }).to_string())
    .execute(pool)
    .await
    .unwrap();
}

pub async fn notes(pool: &SqlitePool, meeting_id: &str, markdown: &str) {
    sqlx::query(
        "INSERT INTO meeting_notes (meeting_id, notes_markdown, created_at, updated_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(meeting_id)
    .bind(markdown)
    .bind(NOW)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn action_item(
    pool: &SqlitePool,
    meeting_id: &str,
    id: &str,
    desc: &str,
    assignee: Option<&str>,
    is_self: bool,
    due: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO action_items (id, meeting_id, description, assignee_person_id,
                                   assignee_is_self, due_hint, status, source, content_key,
                                   created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'open', 'manual', ?, ?, ?)",
    )
    .bind(id)
    .bind(meeting_id)
    .bind(desc)
    .bind(assignee)
    .bind(is_self)
    .bind(due)
    .bind(id)
    .bind(NOW)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

/// Three meetings. `m-pricing` (Nov 3) is the rich one: speakers, a hand-corrected line,
/// participants, summary, notes, action items, seven transcript lines about pricing, and an
/// untimed line. `m-standup` (Nov 1) and `m-retro` (Nov 5) mention pricing once each.
pub async fn fixture(pool: &SqlitePool) {
    meeting(pool, "m-pricing", "Pricing review", "2026-11-03T12:00:00Z").await;
    speaker(pool, "m-pricing", "local", "You").await;
    speaker(pool, "m-pricing", "spk_0", "Dana Ruiz").await;
    speaker(pool, "m-pricing", "spk_1", "Speaker 2").await;
    participant(
        pool,
        "m-pricing",
        "person-dana",
        "Dana Ruiz",
        Some("dana@example.com"),
    )
    .await;
    participant(pool, "m-pricing", "person-lee", "Lee Park", None).await;

    // (id, start, speaker key, text); -1 = no recording time, "" = no speaker key. t6's
    // key has no speakers row.
    let lines = [
        ("t1", 0.0, "local", "Welcome everyone to the review"),
        ("t2", 754.29, "spk_0", "The pricing page needs work"),
        ("t3", 800.0, "spk_1", "Pricing for the pro tier is too low"),
        ("t4", 810.0, "local", "I agree the pricing is off"),
        ("t5", 820.0, "", "Annual pricing could get a discount"),
        ("t6", 830.0, "spk_9", "Pricing experiments next quarter"),
        ("t7", 840.0, "spk_0", "Let us test pricing with a cohort"),
        ("t8", 850.0, "local", "Final word on pricing from me"),
        ("t9", 860.0, "spk_0", "Corrected line about budgets"),
        (
            "t10",
            -1.0,
            "spk_0",
            "An imported untimed remark on budgets",
        ),
    ];
    for (id, start, key, text) in lines {
        let start = (start >= 0.0).then_some(start);
        let key = (!key.is_empty()).then_some(key);
        line(pool, "m-pricing", id, text, start, key).await;
    }
    // t9 is hand-corrected: the diarizer said spk_1 and the user reassigned it to Dana.
    // The correction is applied to transcripts.speaker and kept in the overrides table.
    sqlx::query(
        "INSERT INTO transcript_speaker_overrides (transcript_id, meeting_id, speaker_key, created_at)
         VALUES ('t9', 'm-pricing', 'spk_0', ?)",
    )
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();

    summary(
        pool,
        "m-pricing",
        "## Decisions\n- Raise pricing for the pro tier",
    )
    .await;
    notes(pool, "m-pricing", "Ask finance about pricing tiers").await;
    action_item(
        pool,
        "m-pricing",
        "ai-1",
        "Send the revised price sheet",
        Some("person-dana"),
        false,
        Some("Friday"),
    )
    .await;
    action_item(
        pool,
        "m-pricing",
        "ai-2",
        "Draft the announcement",
        None,
        true,
        None,
    )
    .await;

    meeting(pool, "m-standup", "Daily standup", "2026-11-01T12:00:00Z").await;
    line(
        pool,
        "m-standup",
        "s1",
        "Quick note on pricing before we start",
        Some(5.0),
        None,
    )
    .await;

    meeting(pool, "m-retro", "Sprint retro", "2026-11-05T12:00:00Z").await;
    speaker(pool, "m-retro", "spk_0", "Jordan Ames").await;
    line(
        pool,
        "m-retro",
        "r1",
        "The pricing launch went fine",
        Some(65.0),
        Some("spk_0"),
    )
    .await;
}

pub async fn setup() -> (
    tempfile::TempDir,
    app_lib::database::manager::DatabaseManager,
) {
    let (dir, db) = crate::common::fresh_db().await;
    fixture(db.pool()).await;
    (dir, db)
}

// ---------------------------------------------------------------------------
// specs/0091 — Topics and To-Dos
// ---------------------------------------------------------------------------

pub async fn topic(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    status: &str,
    sort: i64,
    starred: bool,
    hide_from_todo: bool,
) {
    sqlx::query(
        "INSERT INTO topics (id, name, description, status, created_at, updated_at, sort_order,
                             starred, hide_from_todo)
         VALUES (?, ?, '', ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(status)
    .bind(NOW)
    .bind(NOW)
    .bind(sort)
    .bind(starred)
    .bind(hide_from_todo)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn topic_link(pool: &SqlitePool, topic_id: &str, kind: &str, ref_: &str) {
    sqlx::query(
        "INSERT INTO topic_links (topic_id, kind, ref, title, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(topic_id)
    .bind(kind)
    .bind(ref_)
    .bind(ref_)
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn topic_entry(
    pool: &SqlitePool,
    id: &str,
    topic_id: &str,
    meeting_id: Option<&str>,
    occurred_at: &str,
    body: serde_json::Value,
) {
    sqlx::query(
        "INSERT INTO topic_entries (id, topic_id, meeting_id, kind, occurred_at, body_json,
                                    created_at)
         VALUES (?, ?, ?, 'meeting', ?, ?, ?)",
    )
    .bind(id)
    .bind(topic_id)
    .bind(meeting_id)
    .bind(occurred_at)
    .bind(body.to_string())
    .bind(NOW)
    .execute(pool)
    .await
    .unwrap();
}

/// A To-Do: an action item with the given triage/status, Topic, order and highlight.
#[allow(clippy::too_many_arguments)]
pub async fn todo(
    pool: &SqlitePool,
    id: &str,
    meeting_id: Option<&str>,
    desc: &str,
    triage: &str,
    status: &str,
    topic_id: Option<&str>,
    sort: Option<i64>,
    highlight: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO action_items (id, meeting_id, description, assignee_is_self, status,
                                   source, content_key, created_at, updated_at, completed_at,
                                   triage, topic_id, sort_order, highlight)
         VALUES (?, ?, ?, 1, ?, 'manual', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(meeting_id)
    .bind(desc)
    .bind(status)
    .bind(id)
    .bind(NOW)
    .bind(NOW)
    .bind((status == "completed").then_some(NOW))
    .bind(triage)
    .bind(topic_id)
    .bind(sort)
    .bind(highlight)
    .execute(pool)
    .await
    .unwrap();
}

/// Four Topics in manual order: "Project Alpha" (starred, linked to m-pricing and a series,
/// two timeline entries, notes), "Alpine trip", "Hidden work" (kept off the To-Do list) and
/// "Old project" (archived). To-Dos: two on the List (one on Alpha, one with no Topic, the
/// second ordered first), one on Hidden work, one completed on Alpha and one Inbox
/// suggestion that is never on the List.
pub async fn topics_fixture(pool: &SqlitePool) {
    topic(pool, "tp-alpha", "Project Alpha", "open", 0, true, false).await;
    topic(pool, "tp-alps", "Alpine trip", "open", 1, false, false).await;
    topic(pool, "tp-hidden", "Hidden work", "open", 2, false, true).await;
    topic(pool, "tp-old", "Old project", "closed", 3, false, false).await;
    sqlx::query(
        "UPDATE topics SET description = 'Ship the alpha build\nSecond line',
                notes_markdown = 'Remember the launch checklist' WHERE id = 'tp-alpha'",
    )
    .execute(pool)
    .await
    .unwrap();
    topic_link(pool, "tp-alpha", "meeting", "m-pricing").await;
    topic_link(pool, "tp-alpha", "series", "series-weekly").await;
    topic_entry(
        pool,
        "te-1",
        "tp-alpha",
        Some("m-pricing"),
        "2026-11-03T12:00:00Z",
        serde_json::json!({
            "bullets": [{"text": "Pro tier priced too low", "t": 800.0}],
            "decisions": [{"id": "d1", "text": "Raise the pro price", "t": 754.29}]
        }),
    )
    .await;
    topic_entry(
        pool,
        "te-2",
        "tp-alpha",
        None,
        "2026-11-06T12:00:00Z",
        serde_json::json!({"bullets": [{"text": "Hand-written follow-up"}]}),
    )
    .await;

    todo(
        pool,
        "td-alpha",
        Some("m-standup"),
        "Email @[Dana Ruiz](person:person-dana) the plan",
        "listed",
        "open",
        Some("tp-alpha"),
        Some(1),
        Some("yellow"),
    )
    .await;
    todo(
        pool,
        "td-loose",
        None,
        "Book the venue",
        "listed",
        "open",
        None,
        Some(0),
        None,
    )
    .await;
    todo(
        pool,
        "td-hidden",
        None,
        "Private chore",
        "listed",
        "open",
        Some("tp-hidden"),
        Some(2),
        None,
    )
    .await;
    todo(
        pool,
        "td-done",
        None,
        "Finished alpha task",
        "listed",
        "completed",
        Some("tp-alpha"),
        Some(3),
        None,
    )
    .await;
    todo(
        pool,
        "td-inbox",
        None,
        "Suggested thing",
        "suggested",
        "open",
        Some("tp-alpha"),
        None,
        None,
    )
    .await;
}
