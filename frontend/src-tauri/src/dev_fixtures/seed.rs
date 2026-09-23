//! Seeds the fixture dataset through the repositories (specs/0059). Meetings and people are
//! inserted directly so their ids stay stable for the screenshot manifest (0060); every
//! other row goes through the same repository code the app uses.
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveTime, TimeZone, Utc};
use sqlx::SqlitePool;

use super::dataset::{Dataset, FixtureManualMeeting, FixtureMeeting};
use crate::action_items::diff::{content_key, stable_text_fingerprint, ResolvedCandidate};
use crate::aggregation::engine::SourceMeeting;
use crate::aggregation::scope::AggregationScope;
use crate::database::repositories::action_item::ActionItemsRepository;
use crate::database::repositories::ask_ai_history::AskAiHistoryRepository;
use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::database::repositories::summary::SummaryProcessesRepository;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::transcripts::TranscriptSegment;

pub type FolderMap = HashMap<String, PathBuf>;

#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct SeedReport {
    pub meetings: u32,
    pub people: u32,
    pub segments: u32,
    pub failed: u32,
}

/// Tables the seeder owns. Settings, voiceprints and the Keychain are untouched, and
/// so is the connected Google account row — see [`REAL_DATA_CACHE_TABLES`] for what
/// happens to the data that account cached.
const WIPE_TABLES: &[&str] = &[
    "ask_ai_history",
    "action_item_extractions",
    "action_items",
    "meeting_summary_outlines",
    "summary_processes",
    "meeting_notes",
    "meeting_participants",
    "transcript_speaker_overrides",
    "speakers",
    "transcripts",
    "meetings",
    "people",
];

/// Meeting-scoped tables a single failed meeting's rows are removed from by
/// [`remove_meeting_rows`], in delete order. Same set as [`WIPE_TABLES`] minus
/// `ask_ai_history` (not meeting-scoped in the seeder's writes) and `people` (never
/// touched per-meeting). `transcript_speaker_overrides` carries its own `meeting_id`
/// column (migrations/20260701000000_add_transcript_speaker_overrides.sql), so it is
/// deleted the same way as the rest rather than via a `transcripts` subquery.
/// `meetings` itself is deleted LAST, after every table that references it.
const MEETING_SCOPED_TABLES: &[&str] = &[
    "action_item_extractions",
    "action_items",
    "meeting_summary_outlines",
    "summary_processes",
    "meeting_notes",
    "meeting_participants",
    "transcript_speaker_overrides",
    "speakers",
    "transcripts",
];

/// Caches of the developer's *real* data, cleared alongside the seeder's own tables
/// (specs/0060 follow-up).
///
/// A `--demo` profile is the profile README screenshots are captured from, so it has
/// to be fully synthetic. The seeder used to leave these alone on the reasoning that
/// the calendar is not its data to touch — but the first real capture run published
/// 19 real contacts with names, work addresses and face photos, plus real meeting
/// titles on the Today timeline, because these caches survived the wipe and rendered
/// straight through it.
///
/// `google_calendar_account` is deliberately NOT in this list: dropping it would cost
/// a re-authentication. Sync is suppressed at source while the demo dataset is active
/// (`calendar::google::sync`), so an emptied cache stays empty.
const REAL_DATA_CACHE_TABLES: &[&str] = &[
    "google_calendar_events",
    "attendee_photos",
    "dismissed_calendar_events",
    "meeting_briefs",
];

pub async fn wipe(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    for t in WIPE_TABLES {
        sqlx::query(&format!("DELETE FROM {t}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("wipe {t}"))?;
    }
    for t in REAL_DATA_CACHE_TABLES {
        sqlx::query(&format!("DELETE FROM {t}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("wipe {t}"))?;
    }
    // Clearing the events above without clearing the sync tokens would leave the
    // account incrementally in sync against an empty cache: per the schema, a NULL
    // `sync_token` is what forces a full (re)sync, and specs/0054 W5 measured that a
    // full sync otherwise only comes around about every 53 days. Per-calendar
    // `selected` toggles are preserved — the user's choice, not cached data.
    sqlx::query(
        "UPDATE google_calendar_sync
            SET sync_token = NULL, last_synced_at = NULL, window_ends_at = NULL",
    )
    .execute(&mut *tx)
    .await
    .context("reset google_calendar_sync tokens")?;
    tx.commit().await?;
    Ok(())
}

/// Compensating delete for one meeting's rows across every meeting-scoped table, run in a
/// single transaction (repositories take `&SqlitePool`, not a shared transaction handle, so
/// this is the closest available substitute for "the meeting's writes are one transaction").
/// Never touches `people` — people are dataset-wide, not meeting-scoped.
pub async fn remove_meeting_rows(pool: &SqlitePool, meeting_id: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    for t in MEETING_SCOPED_TABLES {
        sqlx::query(&format!("DELETE FROM {t} WHERE meeting_id = ?"))
            .bind(meeting_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("remove {t} rows for {meeting_id}"))?;
    }
    sqlx::query("DELETE FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("remove meetings row for {meeting_id}"))?;
    tx.commit().await?;
    Ok(())
}

/// `days_ago` + `time_of_day` in the machine's local zone, returned as UTC.
pub fn started_at(m: &FixtureMeeting, now: DateTime<Utc>) -> DateTime<Utc> {
    started_at_raw(m.days_ago, &m.time_of_day, now)
}

/// Shared by [`started_at`] and manual-meeting seeding: `days_ago` + `time_of_day`
/// (`"HH:MM"`, falling back to noon on a bad string) in the machine's local zone,
/// returned as UTC.
fn started_at_raw(days_ago: u32, time_of_day: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let t = NaiveTime::parse_from_str(time_of_day, "%H:%M")
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(12, 0, 0).unwrap());
    let day = now.with_timezone(&Local).date_naive() - chrono::Duration::days(days_ago as i64);
    Local
        .from_local_datetime(&day.and_time(t))
        .single()
        .unwrap_or_else(|| Local.from_utc_datetime(&day.and_time(t)))
        .with_timezone(&Utc)
}

/// Seeds the manually-added-meeting fixtures (specs/0069 W3) as `scheduled`-origin rows
/// with a Nixon-minted `calendar_event_id`, exactly the shape
/// `MeetingsRepository::create_manual_scheduled` writes — except the id and event id are
/// the fixture's own stable id rather than a fresh UUID, so re-seeding lands the same rows
/// every time instead of piling up random ones. These are never inserted through
/// `seed_all`/[`insert_meeting_row`]: they carry no transcript and don't participate in its
/// per-meeting rollback bookkeeping. Called from `mod.rs::run_seed` after `seed_all`, so it
/// always runs against a freshly wiped `meetings` table.
pub async fn seed_manual_meetings(
    pool: &SqlitePool,
    manual: &[FixtureManualMeeting],
    now: DateTime<Utc>,
) -> Result<u32> {
    let mut n = 0u32;
    for m in manual {
        let start = started_at_raw(m.days_ago, &m.time_of_day, now);
        let end = start + chrono::Duration::minutes(m.duration_minutes as i64);
        let event_id = format!(
            "{}{}",
            crate::database::repositories::meeting::MANUAL_EVENT_PREFIX,
            m.id
        );
        sqlx::query(
            "INSERT INTO meetings \
             (id, title, created_at, updated_at, origin, calendar_event_id, scheduled_end_at, join_url) \
             VALUES (?, ?, ?, ?, 'scheduled', ?, ?, ?)",
        )
        .bind(&m.id)
        .bind(&m.title)
        .bind(start.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(&event_id)
        .bind(end.to_rfc3339())
        .bind(&m.join_url)
        .execute(pool)
        .await
        .with_context(|| format!("insert manual scheduled row for {}", m.id))?;
        n += 1;
    }
    Ok(n)
}

/// Seeds one example completed Ask-AI run into `ask_ai_history`, through the exact
/// repository write the real success path uses
/// (`aggregation::commands::api_ask_ai_run` → `AskAiHistoryRepository::insert`) — so the
/// `/ask` screenshot shows the app's own citation-chip and sources-list rendering, not a
/// hand-shaped stand-in. Cites recorded fixture meetings by their SEEDED `created_at`
/// (`seed::started_at`, not the fixture's raw `days_ago`/`time_of_day`), matching what the
/// real pipeline would have written for the same run. Must run AFTER `seed_all` — the ids
/// it cites only exist once that's inserted them.
pub async fn seed_ask_ai_history_example(
    pool: &SqlitePool,
    ds: &Dataset,
    now: DateTime<Utc>,
) -> Result<()> {
    let source = |id: &str, title: &str, cited: bool| -> Result<SourceMeeting> {
        let m = ds
            .meetings
            .iter()
            .find(|m| m.id == id)
            .with_context(|| format!("ask-ai history example: no fixture meeting {id}"))?;
        Ok(SourceMeeting {
            meeting_id: id.to_string(),
            title: title.to_string(),
            created_at: started_at(m, now).to_rfc3339(),
            cited,
        })
    };
    // [M#] markers below are 1-indexed into this list (the Rust engine's own numbering) —
    // demo-01 stays last and uncited, mirroring "searched but not cited" in a real run.
    let sources = vec![
        source("demo-04", "Firmware standup", true)?,
        source("demo-06", "Design review — enclosure v3", true)?,
        source("demo-03", "Sensor line steerco", true)?,
        source("demo-01", "Product sync — Q4 firmware", false)?,
    ];
    let answer_markdown = "The September 25 firmware freeze is still on track, but two \
        risks are worth watching.\n\n\
        - **Rev C hardware.** Tomas is waiting on rev C boards to characterize sleep-mode \
        current draw; Inès is targeting Friday but has slipped before, so the team will \
        borrow a board from the Q4 firmware bench if it slips again. Flash usage on the \
        C-series is also at 91% after the new fault-code table — dropping this cycle's \
        diagnostic logging changes and stripping verbose calibration strings should bring \
        it back under 85%. [M1]\n\
        - **Enclosure tooling.** The v3 enclosure design is approved pending a seam-seal \
        test due Thursday. The resulting tool insert change eats most of this quarter's \
        mechanical contingency budget on top of the existing 10-12 week anodizing lead \
        time, leaving no slack for a second design pass. [M2]\n\
        - **Firmware architecture.** Aegis firmware sharing the Vantage codebase behind a \
        new hardware-abstraction layer is de-risked and roughly three weeks out; it \
        doesn't block the freeze. [M3]\n\n\
        Nothing here currently threatens the freeze date itself, but the enclosure tooling \
        has the least room to slip.";
    let scope_json = serde_json::to_string(&AggregationScope::default())
        .context("serialize the example ask-ai scope")?;
    let sources_json =
        serde_json::to_string(&sources).context("serialize the example ask-ai sources")?;

    AskAiHistoryRepository::insert(
        pool,
        "What's currently blocking the Q4 firmware freeze?",
        &scope_json,
        answer_markdown,
        &sources_json,
        Some("builtin-ai"),
        Some("qwen3.5:2b"),
    )
    .await
    .context("insert example ask-ai history row")?;
    Ok(())
}

/// specs/0059 controller ruling (Task 4 review, fix round 1): the parent spec requires
/// "each meeting is one transaction; a failed meeting is rolled back and reported, the
/// others still land". The repositories this seeder calls take `&SqlitePool` (not a
/// shared transaction handle), so a real cross-repository transaction per meeting isn't
/// available without changing their signatures. Instead: the meetings-row INSERT is a
/// separate first step ([`insert_meeting_row`]) whose failure returns early with NO
/// compensating delete — a PRIMARY KEY collision there means the id already belongs to an
/// earlier, successfully seeded meeting, and deleting would destroy that meeting's rows
/// instead of anything this attempt created. Once the row insert succeeds, any later
/// failure ([`seed_meeting_rest`]) is known to be scoped to THIS meeting's fresh rows, so
/// [`remove_meeting_rows`] safely deletes them and seeding continues with the next meeting.
pub async fn seed_all(
    pool: &SqlitePool,
    ds: &Dataset,
    folders: &FolderMap,
    now: DateTime<Utc>,
) -> Result<SeedReport> {
    wipe(pool).await?;
    let mut report = SeedReport::default();
    let mut failures: Vec<String> = Vec::new();
    let ts = now.to_rfc3339();
    for p in &ds.people {
        sqlx::query("INSERT INTO people (id, email, display_name, role, notes, voiceprint_opt_out, created_at, updated_at) VALUES (?, ?, ?, ?, NULL, 0, ?, ?)")
            .bind(&p.id).bind(&p.email).bind(&p.display_name).bind(&p.role).bind(&ts).bind(&ts)
            .execute(pool).await.with_context(|| format!("insert person {}", p.id))?;
        report.people += 1;
    }
    for m in &ds.meetings {
        let start = started_at(m, now);
        let folder_path = folders.get(&m.id).map(|p| p.to_string_lossy().to_string());

        if let Err(e) = insert_meeting_row(pool, m, folder_path.as_deref(), start).await {
            // See the doc comment above: no cleanup here, the row insert itself failed.
            log::error!(
                "[dev] seed {} failed inserting its meetings row: {e:#}; not rolling back \
                 (the id likely belongs to an earlier, successfully seeded meeting)",
                m.id
            );
            report.failed += 1;
            failures.push(format!("{}: {e:#}", m.id));
            continue;
        }

        match seed_meeting_rest(pool, m, folder_path, start).await {
            Ok(n) => {
                report.meetings += 1;
                report.segments += n;
            }
            Err(e) => {
                log::error!("[dev] seed {} failed: {e:#}; rolling back its rows", m.id);
                if let Err(e2) = remove_meeting_rows(pool, &m.id).await {
                    log::error!("[dev] rollback of {} failed too: {e2:#}", m.id);
                }
                report.failed += 1;
                failures.push(format!("{}: {e:#}", m.id));
            }
        }
    }
    if report.meetings == 0 {
        anyhow::bail!("every meeting failed to seed: {}", failures.join("; "));
    }
    Ok(report)
}

async fn insert_meeting_row(
    pool: &SqlitePool,
    m: &FixtureMeeting,
    folder_path: Option<&str>,
    start: DateTime<Utc>,
) -> Result<()> {
    // specs/0072: one meeting per audio state. Speakers count as identified once processed
    // (a `failed` meeting's identification didn't finish).
    let done = matches!(m.audio_state.as_deref(), Some("processed" | "purged"));
    let identified = (done && !m.speakers.is_empty()).then(|| start.to_rfc3339());
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path, origin, template_id, title_manually_set, audio_state, speakers_identified_at) VALUES (?, ?, ?, ?, ?, 'recorded', ?, 1, ?, ?)")
        .bind(&m.id).bind(&m.title).bind(start.to_rfc3339()).bind(start.to_rfc3339()).bind(folder_path).bind(&m.template_id)
        .bind(&m.audio_state).bind(identified)
        .execute(pool).await
        .with_context(|| format!("insert meetings row for {}", m.id))?;
    Ok(())
}

async fn seed_meeting_rest(
    pool: &SqlitePool,
    m: &FixtureMeeting,
    folder_path: Option<String>,
    start: DateTime<Utc>,
) -> Result<u32> {
    for s in &m.speakers {
        SpeakersRepository::upsert(
            pool,
            &m.id,
            &s.key,
            &s.display_name,
            s.is_local,
            None,
            None,
            None,
        )
        .await?;
        if let Some(pid) = &s.person_id {
            sqlx::query(
                "UPDATE speakers SET person_id = ? WHERE meeting_id = ? AND speaker_key = ?",
            )
            .bind(pid)
            .bind(&m.id)
            .bind(&s.key)
            .execute(pool)
            .await?;
        }
    }

    let segments: Vec<TranscriptSegment> = m
        .segments
        .iter()
        .map(|s| TranscriptSegment {
            id: String::new(),
            text: s.text.clone(),
            timestamp: (start + chrono::Duration::milliseconds((s.start * 1000.0) as i64))
                .to_rfc3339(),
            audio_start_time: Some(s.start),
            audio_end_time: Some(s.end),
            duration: Some(s.end - s.start),
            speaker: s.speaker.clone(),
            channel: Some(s.channel.clone()),
            word_timestamps: None,
        })
        .collect();
    let ok = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &m.id,
        &m.title,
        &segments,
        folder_path,
    )
    .await?;
    anyhow::ensure!(ok, "save_transcripts_for_meeting refused {}", m.id);

    for pid in &m.participants {
        MeetingParticipantsRepository::add(pool, &m.id, pid, "calendar").await?;
    }
    if let Some(md) = &m.notes_markdown {
        MeetingNotesRepository::upsert_notes(pool, &m.id, Some(md), None).await?;
    }
    if let Some(md) = &m.summary_markdown {
        SummaryProcessesRepository::create_or_reset_process(pool, &m.id).await?;
        let result = serde_json::json!({
            "markdown": md,
            "summary_status": { "complete": true, "total_chunks": 1, "processed_chunks": 1, "failed_chunks": 0 }
        });
        SummaryProcessesRepository::update_process_completed(
            pool,
            &m.id,
            result,
            1,
            0.0,
            !m.speakers.is_empty(),
            &stable_text_fingerprint(md),
            "",
        )
        .await?;
    }
    for ai in &m.action_items {
        let cand = ResolvedCandidate {
            description: ai.description.clone(),
            assignee_person_id: ai
                .assignee
                .as_deref()
                .filter(|a| *a != "self")
                .map(str::to_string),
            assignee_is_self: ai.assignee.as_deref() == Some("self"),
            assignee_raw: None,
            due_hint: ai.due_hint.clone(),
            due_date: None,
        };
        ActionItemsRepository::create(
            pool,
            Some(&m.id),
            &cand,
            "extracted",
            &content_key(&ai.description),
        )
        .await?;
    }
    Ok(m.segments.len() as u32)
}

#[cfg(test)]
mod manual_tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::memory_db;
    use crate::database::repositories::meeting::{is_manual_event_id, MeetingsRepository};

    fn fixture(id: &str, title: &str, time_of_day: &str, duration_minutes: u32) -> FixtureManualMeeting {
        FixtureManualMeeting {
            id: id.into(),
            title: title.into(),
            days_ago: 0,
            time_of_day: time_of_day.into(),
            duration_minutes,
            join_url: None,
        }
    }

    #[tokio::test]
    async fn seeds_manual_entries_as_scheduled_rows_with_a_stable_nixon_event_id() {
        let pool = memory_db().await;
        let now = chrono::Utc::now();
        let manual = vec![fixture("demo-07", "1:1 with Tomas", "15:30", 30)];

        let n = seed_manual_meetings(&pool, &manual, now).await.unwrap();
        assert_eq!(n, 1);

        let (title, origin, event_id): (String, String, String) = sqlx::query_as(
            "SELECT title, origin, calendar_event_id FROM meetings WHERE id = 'demo-07'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(title, "1:1 with Tomas");
        assert_eq!(origin, "scheduled");
        assert!(is_manual_event_id(&event_id));
        assert_eq!(event_id, "nixon-manual:demo-07");
    }

    #[tokio::test]
    async fn re_seeding_is_idempotent_because_wipe_clears_the_table_first() {
        let pool = memory_db().await;
        let now = chrono::Utc::now();
        let manual = vec![fixture("demo-07", "1:1 with Tomas", "15:30", 30)];

        seed_manual_meetings(&pool, &manual, now).await.unwrap();
        wipe(&pool).await.unwrap();
        seed_manual_meetings(&pool, &manual, now).await.unwrap();

        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meetings WHERE id = 'demo-07'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "a stable id means re-seeding never piles up duplicates");
    }

    #[tokio::test]
    async fn a_seeded_manual_entry_is_a_real_day_agenda_row() {
        // Exercises the SAME repository read the Today agenda uses
        // (`calendar::day_agenda::api_get_day_agenda`), proving the seeded row isn't just a
        // plausible-looking INSERT but actually surfaces as an upcoming, unrecorded entry.
        let pool = memory_db().await;
        let now = chrono::Utc::now();
        let manual = vec![fixture("demo-08", "Roadmap review with Greta", "16:30", 45)];
        seed_manual_meetings(&pool, &manual, now).await.unwrap();

        let start = now
            .with_timezone(&chrono::Local)
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let end = start + chrono::Duration::days(1);
        let rows = MeetingsRepository::get_manual_scheduled_between(&pool, start, end)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "demo-08");
        assert!(rows[0].scheduled_end_at.is_some());
    }
}
