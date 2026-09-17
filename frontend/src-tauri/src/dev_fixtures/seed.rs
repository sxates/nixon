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
}

/// Tables the seeder owns. Settings, calendar, voiceprints, Keychain are untouched.
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

pub async fn wipe(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    for t in WIPE_TABLES {
        sqlx::query(&format!("DELETE FROM {t}"))
            .execute(&mut *tx)
            .await
            .with_context(|| format!("wipe {t}"))?;
    }
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

pub async fn seed_all(
    pool: &SqlitePool,
    ds: &Dataset,
    folders: &FolderMap,
    now: DateTime<Utc>,
) -> Result<SeedReport> {
    wipe(pool).await?;
    let mut report = SeedReport::default();
    let ts = now.to_rfc3339();
    for p in &ds.people {
        sqlx::query("INSERT INTO people (id, email, display_name, role, notes, voiceprint_opt_out, created_at, updated_at) VALUES (?, ?, ?, ?, NULL, 0, ?, ?)")
            .bind(&p.id).bind(&p.email).bind(&p.display_name).bind(&p.role).bind(&ts).bind(&ts)
            .execute(pool).await.with_context(|| format!("insert person {}", p.id))?;
        report.people += 1;
    }
    for m in &ds.meetings {
        let n = seed_meeting(pool, m, folders.get(&m.id), now)
            .await
            .with_context(|| format!("seed {}", m.id))?;
        report.meetings += 1;
        report.segments += n;
    }
    Ok(report)
}

async fn seed_meeting(
    pool: &SqlitePool,
    m: &FixtureMeeting,
    folder: Option<&PathBuf>,
    now: DateTime<Utc>,
) -> Result<u32> {
    let start = started_at(m, now);
    let folder_path = folder.map(|p| p.to_string_lossy().to_string());
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path, origin, template_id, title_manually_set) VALUES (?, ?, ?, ?, ?, 'recorded', ?, 1)")
        .bind(&m.id).bind(&m.title).bind(start.to_rfc3339()).bind(start.to_rfc3339()).bind(&folder_path).bind(&m.template_id)
        .execute(pool).await?;

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
        folder_path.clone(),
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
