//! Topics and To-Dos for the `--demo` profile: one realistic scenario on top of the seeded
//! Meetings, so the Topic pages, the To-Do list (flat and grouped), the Inbox and Done all
//! have something true to show. Runs after `seed_all`, `seed_manual_meetings` and the
//! series linking. Every row is written directly (no LLM jobs run), with fixed ids that
//! `scripts/shots/routes.json` points at. All content is invented.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, Utc};
use sqlx::SqlitePool;

use super::dataset::Dataset;
use crate::action_items::diff::content_key;
use crate::topics::links;

/// Topic tables the scenario owns; `seed::wipe` predates Topics and leaves them alone.
const TOPIC_TABLES: &[&str] = &[
    "topic_revisions",
    "topic_suggestions",
    "topic_entries",
    "topic_links",
    "topic_mention_scans",
    "topics",
];

pub const TOPIC_RELEASE: &str = "topic-q4-release";
pub const TOPIC_ENCLOSURE: &str = "topic-enclosure-v3";
pub const TOPIC_Q1: &str = "topic-q1-planning";
pub const TOPIC_MAYA: &str = "topic-maya-lead";
pub const TOPIC_DRIFT: &str = "topic-solenne-drift";

struct TopicRow {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    notes: Option<&'static str>,
    starred: bool,
    hide_from_todo: bool,
    /// Days ago it was archived; `None` = open.
    closed_days_ago: Option<i64>,
    created_days_ago: i64,
}

const TOPICS: &[TopicRow] = &[
    TopicRow {
        id: TOPIC_RELEASE,
        name: "Q4 firmware release",
        description: "Sleep mode, the calibration patch, BLE diagnostic mode and cloud retry, \
                      from scope lock to the September 25 freeze and GA.",
        notes: Some(
            "## Freeze checklist\n\n\
             - Soak test passes at all three setpoints\n\
             - Solenne beta branch carries the calibration table only\n\
             - Rev C current draw characterized on five units\n\
             - Release notes cover calibration and sleep mode\n\n\
             GA is the Wednesday after freeze week if the week is clean.",
        ),
        starred: true,
        hide_from_todo: false,
        closed_days_ago: None,
        created_days_ago: 22,
    },
    TopicRow {
        id: TOPIC_ENCLOSURE,
        name: "Enclosure v3",
        description: "The anodized aluminum enclosure: screw bosses, gasket seam, tooling \
                      and the bead-blasted demo units for the March trade show.",
        notes: Some(
            "Tool insert is a three-week shop quote on top of 10–12 weeks of anodizing. \
             No slack for a second design pass.",
        ),
        starred: true,
        hide_from_todo: false,
        closed_days_ago: None,
        created_days_ago: 10,
    },
    TopicRow {
        id: TOPIC_Q1,
        name: "Q1 planning",
        description: "Everything deferred out of Q4: flash budget, Northfield's data buffer, \
                      cloud backoff, and whether Aegis shares the Vantage codebase.",
        notes: None,
        starred: true,
        hide_from_todo: false,
        closed_days_ago: None,
        created_days_ago: 1,
    },
    TopicRow {
        id: TOPIC_MAYA,
        name: "Maya's team-lead path",
        description: "Maya's move toward leading the sensor firmware team: the role \
                      proposal, Greta's ramp, and the comp review.",
        notes: Some(
            "Keep the title change and the comp review in the same cycle. \
             Greta runs the calibration validation; Maya owns the ship call.",
        ),
        starred: false,
        hide_from_todo: true,
        closed_days_ago: None,
        created_days_ago: 2,
    },
    TopicRow {
        id: TOPIC_DRIFT,
        name: "Solenne humidity drift",
        description: "Two Solenne Biolabs units reading about four points high on humidity.",
        notes: Some(
            "Root cause: the rev B thermistor reference curve was fitted to an older batch. \
             Fixed in firmware by the corrected lookup table; rev C fixes it in hardware.",
        ),
        starred: false,
        hide_from_todo: false,
        closed_days_ago: Some(6),
        created_days_ago: 21,
    },
];

/// (topic, meeting, kind, bullets [(text, t)], decisions [(id, text, t)])
type Entry = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, Option<f64>)],
    &'static [(&'static str, &'static str, Option<f64>)],
);

const ENTRIES: &[Entry] = &[
    (
        TOPIC_RELEASE,
        "demo-11",
        "series",
        &[
            ("Solenne reported humidity reading about four points high on two units.", None),
            ("Deep-sleep current is around 300µA; field units last nine days against a thirty-day spec.", None),
        ],
        &[("dec-rel-1", "Treat the drift as a firmware problem first and reproduce it on rev B.", None)],
    ),
    (
        TOPIC_RELEASE,
        "demo-12",
        "series",
        &[("Drift confirmed as the stale thermistor reference curve, not the sensor.", None)],
        &[
            ("dec-rel-2", "Q4 scope is locked: sleep mode, calibration patch, BLE diagnostic mode, cloud retry.", None),
            ("dec-rel-3", "Cloud retry stays in scope but has no owner until the rest is sized.", None),
            ("dec-rel-4", "Firmware freeze on September 25.", None),
        ],
    ),
    (
        TOPIC_RELEASE,
        "demo-13",
        "series",
        &[
            ("Corrected table is in a 72-hour soak; day one is under a point at all three setpoints.", None),
            ("Sleep mode measured at about 40µA on rev B, roughly seven times better.", None),
            ("Flash on the C-series build is at 91%.", None),
        ],
        &[("dec-rel-5", "QA writes a manual coverage plan for wake-during-update.", None)],
    ),
    (
        TOPIC_RELEASE,
        "demo-01",
        "series",
        &[
            ("Rev C is through fab with five units due to Tomas by Friday.", None),
            ("Beta branch checked: it only carries the calibration table change.", Some(1484.4)),
        ],
        &[
            ("dec-rel-6", "Freeze holds at September 25; GA the following Wednesday.", None),
            ("dec-rel-7", "The corrected table ships to Solenne first, through a three-day beta.", Some(1113.5)),
            ("dec-rel-8", "Add the connector debounce mitigation and an amber recalibration LED to Q4.", Some(959.1)),
        ],
    ),
    (
        TOPIC_ENCLOSURE,
        "demo-06",
        "series",
        &[
            ("Thicker screw bosses passed ten drop cycles with zero cracks.", None),
            ("Antenna cutout moved to the top edge, clear of the BLE ground plane.", None),
        ],
        &[
            ("dec-enc-1", "Enclosure v3 approved once the widened gasket seam passes a water-spray test.", Some(123.3)),
            ("dec-enc-2", "Sample marketing's darker gray on bead-blast units; no schedule impact.", Some(224.2)),
        ],
    ),
    (
        TOPIC_ENCLOSURE,
        "demo-03",
        "mention",
        &[("Program is yellow: on track for the trade show, with the enclosure as the one real risk.", None)],
        &[(
            "dec-enc-0",
            "Demo units use a bead-blasted finish so the show date doesn't wait on anodizing.",
            Some(357.4),
        )],
    ),
    (
        TOPIC_MAYA,
        "demo-02",
        "series",
        &[("Maya wants to lead the sensor firmware team and mentor Inès and Greta.", None)],
        &[
            ("dec-maya-1", "Maya drafts a proposal for a formal firmware team-lead role.", None),
            ("dec-maya-2", "Greta runs the calibration validation, with Maya reviewing the data.", None),
        ],
    ),
    (
        TOPIC_DRIFT,
        "demo-11",
        "series",
        &[("Two units read about four points high after a week; Solenne caught it against their reference probe.", None)],
        &[("dec-drift-1", "Reproduce on a rev B unit before blaming the sensor.", None)],
    ),
    (
        TOPIC_DRIFT,
        "demo-12",
        "series",
        &[("Swapping the sensor left the drift; swapping the reference table removed it.", None)],
        &[("dec-drift-2", "Ship a corrected lookup table as a firmware patch, no hardware change.", None)],
    ),
];

/// Pending suggestions: (topic, meeting, t, text).
const SUGGESTIONS: &[(&str, &str, f64, &str)] = &[
    (
        TOPIC_Q1,
        "demo-01",
        2090.6,
        "Set up a Q1 planning session covering the flash budget, Northfield's buffer request, \
         cloud backoff and the Aegis codebase question.",
    ),
    (
        TOPIC_Q1,
        "demo-01",
        752.0,
        "Northfield Analytical wants a 24-hour local data buffer, which touches the same code \
         as cloud retry.",
    ),
    (
        TOPIC_ENCLOSURE,
        "demo-03",
        2418.5,
        "Dele will flag it if the enclosure eats the budget contingency past 10%.",
    ),
];

/// A hand-added To-Do: (id, text, topic, due in days, highlight, done days ago).
type Manual = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<i64>,
    Option<&'static str>,
    Option<i64>,
);

/// Adopted from a Meeting: (id, topic, due in days, highlight, notes).
type Adopted = (
    &'static str,
    Option<&'static str>,
    Option<i64>,
    Option<&'static str>,
    Option<&'static str>,
);

const MANUAL: &[Manual] = &[
    ("ai-manual-water-spray", "Review Rafael's water-spray results before the tool insert is released", Some(TOPIC_ENCLOSURE), Some(3), Some("yellow"), None),
    ("ai-manual-beta-scope", "Confirm Solenne's beta build carries only the calibration table", Some(TOPIC_RELEASE), Some(1), Some("green"), None),
    ("ai-manual-release-notes", "Read Maya's draft of the calibration and sleep-mode release notes", Some(TOPIC_RELEASE), Some(4), None, None),
    ("ai-manual-booth", "Ask @[Sun Ji-woo](person:person-sun) to walk me through the trade show booth plan", None, None, None, None),
    ("ai-manual-flash", "Pull the flash usage numbers for the C-series build", Some(TOPIC_Q1), None, None, None),
    ("ai-manual-expenses", "Submit the Corvant site-visit expenses", None, Some(-1), None, None),
    ("ai-manual-comp", "Put Maya's title change and comp review in the same cycle", Some(TOPIC_MAYA), Some(9), None, None),
    ("ai-manual-greta", "Send Greta the soak-test handoff notes", Some(TOPIC_MAYA), None, None, None),
    ("ai-manual-scope-mail", "Send the locked Q4 scope to Greta Voss", Some(TOPIC_RELEASE), None, None, Some(13)),
    ("ai-manual-serials", "Log the Solenne serial numbers in the incident ticket", Some(TOPIC_DRIFT), None, None, Some(19)),
];

fn day(offset: i64) -> String {
    (Local::now().date_naive() + Duration::days(offset))
        .format("%Y-%m-%d")
        .to_string()
}

fn ago(now: DateTime<Utc>, days: i64) -> String {
    (now - Duration::days(days)).to_rfc3339()
}

fn body_json(
    bullets: &[(&str, Option<f64>)],
    decisions: &[(&str, &str, Option<f64>)],
) -> String {
    serde_json::json!({
        "bullets": bullets.iter().map(|(text, t)| serde_json::json!({"text": text, "t": t})).collect::<Vec<_>>(),
        "decisions": decisions.iter().map(|(id, text, t)| serde_json::json!({"id": id, "text": text, "t": t})).collect::<Vec<_>>(),
    })
    .to_string()
}

pub async fn seed_topics_and_todos(
    pool: &SqlitePool,
    ds: &Dataset,
    now: DateTime<Utc>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for t in TOPIC_TABLES {
        sqlx::query(&format!("DELETE FROM {t}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("wipe {t}"))?;
    }
    // `seed::wipe` predates the 0088 side tables: drop what no longer has an item.
    for t in ["action_item_sources", "action_item_people"] {
        sqlx::query(&format!(
            "DELETE FROM {t} WHERE action_item_id NOT IN (SELECT id FROM action_items)"
        ))
        .execute(&mut *tx)
        .await
        .with_context(|| format!("prune {t}"))?;
    }

    // Fixed ids for the extracted items, `ai-<meeting>-<index>` like the headless mock.
    for m in &ds.meetings {
        for (i, ai) in m.action_items.iter().enumerate() {
            let old: Option<String> = sqlx::query_scalar(
                "SELECT id FROM action_items WHERE meeting_id = ? AND description = ?",
            )
            .bind(&m.id)
            .bind(&ai.description)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(old) = old else { continue };
            let new = format!("ai-{}-{i}", m.id);
            for sql in [
                "UPDATE action_items SET id = ?1 WHERE id = ?2",
                "UPDATE action_item_sources SET action_item_id = ?1 WHERE action_item_id = ?2",
            ] {
                sqlx::query(sql).bind(&new).bind(&old).execute(&mut *tx).await?;
            }
        }
    }

    for (i, t) in TOPICS.iter().enumerate() {
        let created = ago(now, t.created_days_ago);
        sqlx::query(
            "INSERT INTO topics (id, name, description, status, notes_markdown, created_at,
                                 updated_at, closed_at, sort_order, starred, hide_from_todo)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(t.id)
        .bind(t.name)
        .bind(t.description)
        .bind(if t.closed_days_ago.is_some() { "closed" } else { "open" })
        .bind(t.notes)
        .bind(&created)
        .bind(&created)
        .bind(t.closed_days_ago.map(|d| ago(now, d)))
        .bind(i as i64)
        .bind(t.starred)
        .bind(t.hide_from_todo)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert topic {}", t.id))?;
    }

    for (topic, meeting) in [
        (TOPIC_ENCLOSURE, "demo-06"),
        (TOPIC_MAYA, "demo-02"),
        (TOPIC_DRIFT, "demo-11"),
        (TOPIC_DRIFT, "demo-12"),
    ] {
        sqlx::query(
            "INSERT INTO topic_links (topic_id, kind, ref, title, created_at)
             SELECT ?, 'meeting', id, NULL, ? FROM meetings WHERE id = ?",
        )
        .bind(topic)
        .bind(now.to_rfc3339())
        .bind(meeting)
        .execute(&mut *tx)
        .await?;
    }

    for (n, (topic, meeting, kind, bullets, decisions)) in ENTRIES.iter().enumerate() {
        let json = body_json(bullets, decisions);
        sqlx::query(
            "INSERT INTO topic_entries
                (id, topic_id, meeting_id, kind, occurred_at, ai_body_json, body_json, created_at)
             SELECT ?, ?, id, ?, created_at, ?, ?, created_at FROM meetings WHERE id = ?",
        )
        .bind(format!("te-demo-{n}"))
        .bind(topic)
        .bind(kind)
        .bind(&json)
        .bind(&json)
        .bind(meeting)
        .execute(&mut *tx)
        .await?;
    }
    // Pre-freeze moved a scope-lock decision: cloud retry waits for Q1 planning.
    sqlx::query(
        "INSERT INTO topic_revisions
            (id, entry_id, decision_id, text, source_meeting_id, t_start, created_at)
         SELECT 'tr-demo-0', 'te-demo-1', 'dec-rel-3',
                'Cloud retry is deferred to the Q1 planning session.', id, 2090.6, created_at
         FROM meetings WHERE id = 'demo-01'",
    )
    .execute(&mut *tx)
    .await?;

    for (n, (topic, meeting, t, text)) in SUGGESTIONS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO topic_suggestions
                (id, topic_id, kind, meeting_id, t_start, text, status, created_at)
             SELECT ?, ?, 'note', id, ?, ?, 'pending', created_at FROM meetings WHERE id = ?",
        )
        .bind(format!("ts-demo-{n}"))
        .bind(topic)
        .bind(t)
        .bind(text)
        .bind(meeting)
        .execute(&mut *tx)
        .await?;
    }

    // ---- To-Dos ----
    // The BLE follow-up came up in two product syncs: one item, both Meetings as sources.
    sqlx::query(
        "INSERT OR IGNORE INTO action_item_sources (action_item_id, meeting_id, context, phrasing, added_at)
         SELECT 'ai-demo-01-0', meeting_id, 'Mobile has not committed to a date yet.', description, created_at
         FROM action_items WHERE id = 'ai-demo-12-0'",
    )
    .execute(&mut *tx)
    .await?;
    for sql in [
        "DELETE FROM action_item_sources WHERE action_item_id = 'ai-demo-12-0'",
        "DELETE FROM action_items WHERE id = 'ai-demo-12-0'",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }

    let adopted: &[Adopted] = &[
        (
            "ai-demo-01-0",
            Some(TOPIC_RELEASE),
            Some(2),
            Some("yellow"),
            Some("Mobile's last estimate was \"after their 4.2 release\". Ask for a date, not a release."),
        ),
        ("ai-demo-01-4", Some(TOPIC_Q1), Some(6), None, None),
    ];
    // The List, top to bottom.
    let listed: &[&str] = &[
        "ai-demo-01-0",
        "ai-manual-water-spray",
        "ai-manual-beta-scope",
        "ai-manual-release-notes",
        "ai-demo-01-4",
        "ai-manual-flash",
        "ai-manual-booth",
        "ai-manual-expenses",
        "ai-manual-comp",
        "ai-manual-greta",
    ];
    let slot = |id: &str| listed.iter().position(|l| *l == id).map(|p| p as i64);

    for (id, topic, due, highlight, notes) in adopted {
        sqlx::query(
            "UPDATE action_items SET triage = 'listed', adopted_at = ?, sort_order = ?,
                    topic_id = ?, due_date = ?, highlight = ?, notes_markdown = ?, user_edited = 1
             WHERE id = ?",
        )
        .bind(ago(now, 0))
        .bind(slot(id))
        .bind(topic)
        .bind(due.map(day))
        .bind(highlight)
        .bind(notes)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    // Finished items of mine from past Meetings show in Done, on the drift Topic.
    sqlx::query(
        "UPDATE action_items SET triage = 'listed', adopted_at = created_at, topic_id = ?
         WHERE status = 'completed' AND assignee_is_self = 1",
    )
    .bind(TOPIC_DRIFT)
    .execute(&mut *tx)
    .await?;

    for (id, text, topic, due, highlight, done) in MANUAL {
        let created = ago(now, done.map_or(2, |d| d + 2));
        sqlx::query(
            "INSERT INTO action_items
                (id, meeting_id, description, assignee_is_self, status, source, user_edited,
                 content_key, created_at, updated_at, completed_at, triage, adopted_at,
                 sort_order, topic_id, due_date, highlight)
             VALUES (?, NULL, ?, 1, ?, 'manual', 0, ?, ?, ?, ?, 'listed', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(text)
        .bind(if done.is_some() { "completed" } else { "open" })
        .bind(content_key(text))
        .bind(&created)
        .bind(&created)
        .bind(done.map(|d| ago(now, d)))
        .bind(&created)
        .bind(slot(id))
        .bind(topic)
        .bind(due.map(day))
        .bind(highlight)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert to-do {id}"))?;
    }
    sqlx::query(
        "INSERT INTO action_item_people (action_item_id, person_id)
         SELECT 'ai-manual-booth', id FROM people WHERE id = 'person-sun'",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // The release Topic follows the whole Product sync Series (its key is minted at seed).
    links::link_series_of(pool, TOPIC_RELEASE, "demo-01")
        .await
        .context("link the release Topic to the Product sync Series")?;
    Ok(())
}
