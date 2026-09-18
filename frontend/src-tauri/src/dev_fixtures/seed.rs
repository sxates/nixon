//! Seeds the fixture dataset through the repositories (specs/0059). Meetings and people are
//! inserted directly so their ids stay stable for the screenshot manifest (0060); every
//! other row goes through the same repository code the app uses.
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveTime, TimeZone, Utc};
use sqlx::SqlitePool;

use super::dataset::{Dataset, FixtureMeeting};
use crate::action_items::diff::{content_key, stable_text_fingerprint, ResolvedCandidate};
use crate::database::repositories::action_item::ActionItemsRepository;
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
    let t = NaiveTime::parse_from_str(&m.time_of_day, "%H:%M")
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(12, 0, 0).unwrap());
    let day = now.with_timezone(&Local).date_naive() - chrono::Duration::days(m.days_ago as i64);
    Local
        .from_local_datetime(&day.and_time(t))
        .single()
        .unwrap_or_else(|| Local.from_utc_datetime(&day.and_time(t)))
        .with_timezone(&Utc)
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
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path, origin, template_id, title_manually_set) VALUES (?, ?, ?, ?, ?, 'recorded', ?, 1)")
        .bind(&m.id).bind(&m.title).bind(start.to_rfc3339()).bind(start.to_rfc3339()).bind(folder_path).bind(&m.template_id)
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
