//! Seeds the fixture dataset through the repositories (specs/0059). Meetings and people are
//! inserted directly so their ids stay stable for the screenshot manifest (0060); every
//! other row goes through the same repository code the app uses.
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveTime, TimeZone, Utc};
use sqlx::SqlitePool;

use super::dataset::{add_working_days, Dataset, FixtureManualMeeting, FixtureMeeting};
use crate::action_items::diff::{content_key, stable_text_fingerprint, ResolvedCandidate};
use crate::aggregation::engine::SourceMeeting;
use crate::aggregation::scope::AggregationScope;
use crate::database::repositories::action_item::ActionItemsRepository;
use crate::database::repositories::ask_ai_history::AskAiHistoryRepository;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::meeting_brief::MeetingBriefsRepository;
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
    "summary_processes",
    "meeting_notes",
    "meeting_participants",
    "transcript_speaker_overrides",
    "speakers",
    "transcripts",
    "meeting_series_links",
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
    "summary_processes",
    "meeting_notes",
    "meeting_participants",
    "transcript_speaker_overrides",
    "speakers",
    "transcripts",
    "meeting_series_links",
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
    let day = now.with_timezone(&Local).date_naive() - chrono::Duration::days(days_ago as i64);
    local_start(day, time_of_day)
}

fn local_start(day: chrono::NaiveDate, time_of_day: &str) -> DateTime<Utc> {
    let t = NaiveTime::parse_from_str(time_of_day, "%H:%M")
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(12, 0, 0).unwrap());
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
        let start = if m.days_ahead > 0 {
            let today = now.with_timezone(&Local).date_naive();
            local_start(add_working_days(today, m.days_ahead), &m.time_of_day)
        } else {
            started_at_raw(m.days_ago, &m.time_of_day, now)
        };
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
    let mut seeded: std::collections::HashSet<&str> = std::collections::HashSet::new();
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
                seeded.insert(m.id.as_str());
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
    // Every recorded meeting is in, so series link targets and brief sources exist.
    seed_series_and_briefs(pool, ds, &seeded).await?;
    Ok(report)
}

/// Link recurring occurrences into manual series and write ready pre-call briefs
/// (specs/0082). Runs after every meeting is seeded. Meetings that failed to seed are
/// skipped; any other failure carries the meeting id.
async fn seed_series_and_briefs(
    pool: &SqlitePool,
    ds: &Dataset,
    seeded: &std::collections::HashSet<&str>,
) -> Result<()> {
    // Group recorded occurrences by series; the latest occurrence is the link target.
    let mut by_series: HashMap<&str, Vec<&FixtureMeeting>> = HashMap::new();
    for m in ds.meetings.iter().filter(|m| seeded.contains(m.id.as_str())) {
        if let Some(s) = &m.series {
            by_series.entry(s).or_default().push(m);
        }
    }
    for (series, mut occ) in by_series {
        // Oldest first: the latest occurrence (smallest days_ago, then latest time) is last.
        occ.sort_by_key(|m| (std::cmp::Reverse(m.days_ago), m.time_of_day.clone()));
        let Some(target) = occ.last().copied() else {
            continue;
        };
        for m in occ.iter().filter(|m| m.id != target.id) {
            MeetingsRepository::link_meeting_to_series(pool, &m.id, &target.id)
                .await
                .with_context(|| format!("link {} into series {series}", m.id))?;
        }
    }

    for m in ds.meetings.iter().filter(|m| seeded.contains(m.id.as_str())) {
        let Some(md) = m.brief_markdown.as_deref() else {
            continue;
        };
        // A brief citing a meeting that failed to seed would point its chips at nothing;
        // skip it like the failed meeting itself rather than abort the whole seed.
        if let Some(missing) = m.brief_sources.iter().find(|id| !seeded.contains(id.as_str())) {
            log::error!(
                "[dev] skipping brief for {}: its source {missing} failed to seed",
                m.id
            );
            continue;
        }
        if let Err(e) = seed_brief(pool, m, md).await {
            log::error!("[dev] seed brief for {} failed: {e:#}", m.id);
        }
    }
    Ok(())
}

async fn seed_brief(pool: &SqlitePool, m: &FixtureMeeting, md: &str) -> Result<()> {
    let meta = MeetingsRepository::get_meeting_metadata(pool, &m.id)
        .await?
        .context("seeded meeting missing")?;
    let key = MeetingsRepository::resolve_effective_series_key(pool, &m.id).await?;
    let prior = MeetingsRepository::find_prior_series_occurrences(
        pool,
        key.as_deref(),
        &meta.title,
        meta.created_at.0,
        crate::aggregation::prep_jobs::MAX_PRIOR,
    )
    .await?;
    if prior != m.brief_sources {
        log::warn!(
            "[dev] fixture {}: brief_sources {:?} differ from the app's prior set {:?}",
            m.id,
            m.brief_sources,
            prior
        );
    }
    // Fingerprint the app's own prior set so the seeded brief is `UpToDate` to `plan_brief`.
    let fingerprint = crate::aggregation::prep_jobs::source_fingerprint(pool, &prior).await;
    // Same shape the generator stores (`answer.sources`): `[M{i+1}]` <-> sources[i]. The
    // generator stores the markdown with raw `[M#]` markers (postprocess_citations only
    // normalizes them), so the fixture text is stored as is.
    let mut sources = Vec::with_capacity(m.brief_sources.len());
    for (i, id) in m.brief_sources.iter().enumerate() {
        let sm = MeetingsRepository::get_meeting_metadata(pool, id)
            .await?
            .with_context(|| format!("brief source {id} missing"))?;
        sources.push(SourceMeeting {
            meeting_id: id.clone(),
            title: sm.title,
            created_at: sm.created_at.0.to_rfc3339(),
            cited: md.contains(&format!("[M{}]", i + 1)),
        });
    }
    MeetingBriefsRepository::upsert_ready(
        pool,
        &m.id,
        md,
        &serde_json::to_string(&sources)?,
        &fingerprint,
        "fixture",
        "demo",
    )
    .await?;
    Ok(())
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
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at, folder_path, origin, title_manually_set, audio_state, speakers_identified_at) VALUES (?, ?, ?, ?, ?, 'recorded', 1, ?, ?)")
        .bind(&m.id).bind(&m.title).bind(start.to_rfc3339()).bind(start.to_rfc3339()).bind(folder_path)
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
    if let Some(md) = &m.prep_markdown {
        MeetingNotesRepository::upsert_prep_notes(pool, &m.id, Some(md), None).await?;
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
        let item = ActionItemsRepository::create(
            pool,
            Some(&m.id),
            &cand,
            "extracted",
            &content_key(&ai.description),
        )
        .await?;
        if ai.status.as_deref() == Some("completed") {
            ActionItemsRepository::set_status(pool, &item.id, "completed").await?;
        }
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
            days_ahead: 0,
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

#[cfg(test)]
mod series_tests {
    use super::*;
    use crate::aggregation::prep_jobs::{self, BriefPlan};
    use crate::database::models::MeetingNote;
    use crate::database::repositories::meeting::test_support::memory_db;

    fn meeting(id: &str, days_ago: u32) -> FixtureMeeting {
        FixtureMeeting {
            id: id.into(),
            title: "Product sync".into(),
            days_ago,
            time_of_day: "10:00".into(),
            duration_seconds: 1800,
            audio_state: None,
            participants: vec![],
            speakers: vec![],
            segments: vec![],
            summary_markdown: Some(format!("Summary of {id}")),
            notes_markdown: None,
            action_items: vec![],
            series: Some("product-sync".into()),
            prep_markdown: None,
            brief_markdown: None,
            brief_sources: vec![],
        }
    }

    fn item(description: &str, status: Option<&str>) -> crate::dev_fixtures::dataset::FixtureActionItem {
        crate::dev_fixtures::dataset::FixtureActionItem {
            description: description.into(),
            assignee: Some("self".into()),
            due_hint: None,
            status: status.map(str::to_string),
        }
    }

    fn dataset() -> Dataset {
        let mut prior1 = meeting("p1", 14);
        prior1.action_items = vec![item("Send the deck", Some("completed")), item("Book the room", None)];
        let prior2 = meeting("p2", 7);
        let mut target = meeting("t", 0);
        target.prep_markdown = Some("- Cover the launch date".into());
        target.brief_markdown = Some("Last time [M1] and before that [M2].".into());
        target.brief_sources = vec!["p2".into(), "p1".into()];
        Dataset {
            people: vec![],
            meetings: vec![prior1, prior2, target],
            manual_meetings: vec![],
        }
    }

    async fn seeded() -> (SqlitePool, DateTime<Utc>) {
        let pool = memory_db().await;
        let now = Utc::now();
        seed_all(&pool, &dataset(), &FolderMap::new(), now).await.unwrap();
        (pool, now)
    }

    async fn priors(pool: &SqlitePool) -> Vec<String> {
        let meta = MeetingsRepository::get_meeting_metadata(pool, "t").await.unwrap().unwrap();
        let key = MeetingsRepository::resolve_effective_series_key(pool, "t").await.unwrap();
        MeetingsRepository::find_prior_series_occurrences(pool, key.as_deref(), &meta.title, meta.created_at.0, 10)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn links_group_occurrences_into_one_series_newest_first() {
        let (pool, _) = seeded().await;
        assert_eq!(priors(&pool).await, vec!["p2".to_string(), "p1".to_string()]);
    }

    #[tokio::test]
    async fn seeds_a_ready_brief_with_cited_sources() {
        let (pool, _) = seeded().await;
        let b = MeetingBriefsRepository::get(&pool, "t").await.unwrap().unwrap();
        assert_eq!(b.status, "ready");
        assert_eq!(b.brief_markdown.as_deref(), Some("Last time [M1] and before that [M2]."));
        let sources: Vec<SourceMeeting> = serde_json::from_str(b.sources_json.as_deref().unwrap()).unwrap();
        assert_eq!(sources.iter().map(|s| s.meeting_id.as_str()).collect::<Vec<_>>(), ["p2", "p1"]);
        assert!(sources.iter().all(|s| s.cited));
    }

    #[tokio::test]
    async fn the_seeded_brief_is_up_to_date_for_the_app_planner() {
        let (pool, _) = seeded().await;
        let plan = prep_jobs::plan_brief(&pool, "t", false).await.unwrap();
        assert_eq!(plan, BriefPlan::UpToDate);
    }

    #[tokio::test]
    async fn prep_notes_and_item_status_are_seeded() {
        let (pool, _) = seeded().await;
        let notes: MeetingNote = MeetingNotesRepository::get_notes(&pool, "t").await.unwrap().unwrap();
        assert_eq!(notes.prep_markdown.as_deref(), Some("- Cover the launch date"));
        let open = ActionItemsRepository::get_open_for_meetings(&pool, &priors(&pool).await)
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].description, "Book the room");
    }

    #[tokio::test]
    async fn re_seeding_leaves_one_link_row_per_occurrence() {
        let (pool, now) = seeded().await;
        seed_all(&pool, &dataset(), &FolderMap::new(), now).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meeting_series_links")
            .fetch_one(&pool)
            .await
            .unwrap();
        // link_meeting_to_series also pins the target itself, so 3 rows for 3 occurrences.
        assert_eq!(n, 3);
    }

    #[tokio::test]
    async fn a_brief_whose_source_failed_to_seed_is_skipped_not_fatal() {
        let pool = memory_db().await;
        // Fail p1's summary write, after its meetings row went in, so p1 is rolled back.
        sqlx::query(
            "CREATE TRIGGER fail_p1 BEFORE INSERT ON summary_processes WHEN NEW.meeting_id = 'p1' \
             BEGIN SELECT RAISE(ABORT, 'boom'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let report = seed_all(&pool, &dataset(), &FolderMap::new(), Utc::now()).await.unwrap();
        assert_eq!((report.meetings, report.failed), (2, 1));
        assert!(MeetingBriefsRepository::get(&pool, "t").await.unwrap().is_none());
    }
}
