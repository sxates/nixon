use crate::transcripts::TranscriptSegment;
use chrono::Utc;
use sqlx::{Connection, Error as SqlxError, SqliteConnection, SqlitePool};
use tracing::{error, info};
use uuid::Uuid;

pub struct TranscriptsRepository;

/// Inserts every transcript segment for `meeting_id` within an existing
/// transaction. Shared by `save_transcript` (new meeting),
/// `save_transcripts_for_meeting` (existing meeting created at recording start),
/// and `append_transcripts_for_meeting` (specs/0037 resume — a second session).
///
/// `audio_offset` (seconds) is added to each segment's recording-relative
/// `audio_start_time`/`audio_end_time` so a resumed session's clock (which
/// restarts at 0) lands after the prior audio on the meeting's continuous
/// timeline. It is `0.0` for a first/only session.
async fn insert_segments(
    transaction: &mut SqliteConnection,
    meeting_id: &str,
    transcripts: &[TranscriptSegment],
    audio_offset: f64,
) -> Result<(), SqlxError> {
    for segment in transcripts {
        let transcript_id = format!("transcript-{}", Uuid::new_v4());
        // specs/0029 WS3.4: capture-channel tag. The payload round-trips through the
        // frontend, so allowlist the known values defensively — anything else is
        // stored as NULL (same as legacy rows) rather than trusted verbatim.
        let channel = segment
            .channel
            .as_deref()
            .filter(|c| matches!(*c, "microphone" | "system" | "mixed"));
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker, channel, word_timestamps)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&transcript_id)
        .bind(meeting_id)
        .bind(&segment.text)
        .bind(&segment.timestamp)
        .bind(segment.audio_start_time.map(|t| t + audio_offset))
        .bind(segment.audio_end_time.map(|t| t + audio_offset))
        .bind(segment.duration)
        // Diarization runs post-meeting (specs/0010), so this is NULL at save time
        // for live recordings; the diarization pipeline sets it later. Preserve any
        // value the caller supplied (e.g. re-import).
        .bind(&segment.speaker)
        .bind(channel)
        // specs/0046 WS2: per-word JSON, NULL for Whisper/legacy segments.
        .bind(&segment.word_timestamps)
        .execute(&mut *transaction)
        .await
        .inspect_err(|e| {
            error!(
                "Failed to save transcript segment for meeting {}: {}",
                meeting_id, e
            )
        })?;
    }
    Ok(())
}

impl TranscriptsRepository {
    /// Saves a new meeting and its associated transcript segments.
    /// This function uses a transaction to ensure that either both the meeting
    /// and all its transcripts are saved, or none of them are.
    pub async fn save_transcript(
        pool: &SqlitePool,
        meeting_title: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now();

        let result = sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(meeting_title)
        .bind(now)
        .bind(now)
        .bind(&folder_path)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = result {
            error!("Failed to create meeting '{}': {}", meeting_title, e);
            transaction.rollback().await?;
            return Err(e);
        }

        info!("Successfully created meeting with id: {}", meeting_id);

        if let Err(e) = insert_segments(&mut transaction, &meeting_id, transcripts, 0.0).await {
            transaction.rollback().await?;
            return Err(e);
        }

        info!(
            "Successfully saved {} transcript segments for meeting {}",
            transcripts.len(),
            meeting_id
        );

        transaction.commit().await?;

        Ok(meeting_id)
    }

    /// Attaches transcript segments to an EXISTING meeting (created at recording
    /// start via [`super::meeting::MeetingsRepository::create_meeting`]) and
    /// refreshes its `title`/`updated_at` (and `folder_path` when provided).
    /// Rows with an authoritative title — calendar-linked (`calendar_event_id`)
    /// or user-renamed (`title_manually_set`) — keep it; only `updated_at` (and
    /// `folder_path`) are refreshed for those.
    ///
    /// Returns `Ok(true)` when the segments were attached to the existing (empty)
    /// meeting row. Returns `Ok(false)` — meaning "did NOT attach; caller should
    /// create a fresh meeting for these segments" — in two cases:
    /// 1. no meeting with `meeting_id` exists (e.g. it was deleted), or
    /// 2. the meeting **already holds transcripts from a recording session**.
    ///
    /// Case 2 is the safety net for `specs/0019` WS6.7: the persist-at-start
    /// contract is exactly one save per meeting (the row is created empty at
    /// recording start and populated once at stop). If a *second* session's stop
    /// hands us a stale `meeting_id` that already has transcripts, blindly
    /// appending would interleave two meetings' transcripts AND overwrite the
    /// existing row's `title`/`folder_path` (while preserving its
    /// `calendar_event_id` → wrong attendees). Instead we refuse to touch the
    /// populated row and signal the caller to mint a new meeting, so each session
    /// keeps its own row, audio, and (absent) calendar link. This turns silent
    /// cross-meeting corruption into a clean new row + a loud warning.
    pub async fn save_transcripts_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
        meeting_title: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
    ) -> Result<bool, SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now();

        // 0. specs/0019 WS6.7 — refuse to merge: if this meeting already holds a
        // session's transcripts, do NOT append/overwrite it. Check BEFORE the
        // UPDATE so the existing row's title/folder_path/calendar_event_id are left
        // untouched. Return Ok(false) so the caller saves this session to a new row.
        let existing_segments: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
                .bind(meeting_id)
                .fetch_one(&mut *transaction)
                .await?;
        if existing_segments > 0 {
            transaction.rollback().await?;
            error!(
                "Refusing to append {} segments to meeting {} which already has {} \
                 transcript(s) from another session (specs/0019 WS6.7); caller will \
                 create a new meeting instead.",
                transcripts.len(),
                meeting_id,
                existing_segments
            );
            return Ok(false);
        }

        // 1. Update the existing meeting's title/updated_at (+ folder_path if given).
        // rows_affected == 0 means the meeting does not exist -> signal the caller.
        //
        // The title is only refreshed when the row does NOT already carry an
        // authoritative one: a calendar-linked meeting keeps its event title and a
        // manually-renamed meeting keeps the user's title (same authority rule as
        // the summary auto-titler, see summary/service.rs). Without this, the stop
        // path's session name overwrote the calendar event title while leaving
        // `calendar_event_id` intact — a date-stamped meeting with correct attendees.
        let keep_title: Option<bool> = sqlx::query_scalar(
            "SELECT (calendar_event_id IS NOT NULL AND calendar_event_id != '')
                    OR title_manually_set = 1
             FROM meetings WHERE id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let keep_title = keep_title.unwrap_or(false);

        let update =
            match (keep_title, folder_path.is_some()) {
                (false, true) => sqlx::query(
                    "UPDATE meetings SET title = ?, updated_at = ?, folder_path = ? WHERE id = ?",
                )
                .bind(meeting_title)
                .bind(now)
                .bind(&folder_path)
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?,
                (false, false) => {
                    sqlx::query("UPDATE meetings SET title = ?, updated_at = ? WHERE id = ?")
                        .bind(meeting_title)
                        .bind(now)
                        .bind(meeting_id)
                        .execute(&mut *transaction)
                        .await?
                }
                (true, true) => {
                    sqlx::query("UPDATE meetings SET updated_at = ?, folder_path = ? WHERE id = ?")
                        .bind(now)
                        .bind(&folder_path)
                        .bind(meeting_id)
                        .execute(&mut *transaction)
                        .await?
                }
                (true, false) => {
                    sqlx::query("UPDATE meetings SET updated_at = ? WHERE id = ?")
                        .bind(now)
                        .bind(meeting_id)
                        .execute(&mut *transaction)
                        .await?
                }
            };

        if update.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }

        if let Err(e) = insert_segments(&mut transaction, meeting_id, transcripts, 0.0).await {
            transaction.rollback().await?;
            return Err(e);
        }

        transaction.commit().await?;

        info!(
            "Attached {} transcript segments to existing meeting {}",
            transcripts.len(),
            meeting_id
        );

        Ok(true)
    }

    /// Appends a RESUMED session's transcript segments to an existing meeting
    /// (specs/0037). Unlike [`Self::save_transcripts_for_meeting`], this
    /// deliberately does NOT refuse a populated meeting — a resume is an
    /// *intentional* second session, not the WS6.7 duplicate-row race.
    ///
    /// Each new segment's recording-relative audio times are shifted by
    /// `audio_offset_seconds` (the meeting's prior audio duration) so the combined
    /// timeline stays continuous and aligned with the concatenated `system.wav`
    /// the offline diarization pass reads. Refreshes `updated_at` (+ `folder_path`
    /// when given); never rewrites the title — the meeting already has its identity
    /// from the first session.
    ///
    /// Returns `Ok(false)` only when no meeting with `meeting_id` exists.
    pub async fn append_transcripts_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
        audio_offset_seconds: f64,
    ) -> Result<bool, SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;
        let now = Utc::now();

        let update = if folder_path.is_some() {
            sqlx::query("UPDATE meetings SET updated_at = ?, folder_path = ? WHERE id = ?")
                .bind(now)
                .bind(&folder_path)
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?
        } else {
            sqlx::query("UPDATE meetings SET updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?
        };
        if update.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }

        // specs/0037 idempotency: the append path deliberately bypasses the WS6.7
        // duplicate-row guard, but a resumed stop can still double-fire (React strict
        // mode, a retry, racing stop handlers). Dedupe on the batch's own CONTENT — a
        // repeat delivers the identical batch, so its first segment's (timestamp, text)
        // already existing for this meeting means this exact batch already landed.
        // Content-exact beats the earlier positional (audio_start_time >= offset)
        // heuristic, which silently dropped legitimate appends when the offset
        // under-counted (None segment durations), was blind to NULL start times, and
        // had to be disabled at offset 0.0 — the crash-recovery case.
        if let Some(first) = transcripts.first() {
            let already_appended: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM transcripts \
                 WHERE meeting_id = ? AND timestamp = ? AND transcript = ? LIMIT 1)",
            )
            .bind(meeting_id)
            .bind(&first.timestamp)
            .bind(&first.text)
            .fetch_one(&mut *transaction)
            .await?;
            if already_appended {
                transaction.rollback().await?;
                info!(
                    "append: meeting {} already contains this batch's first segment — \
                     skipping duplicate resume append",
                    meeting_id
                );
                return Ok(true);
            }
        }

        if let Err(e) = insert_segments(
            &mut transaction,
            meeting_id,
            transcripts,
            audio_offset_seconds,
        )
        .await
        {
            transaction.rollback().await?;
            return Err(e);
        }

        transaction.commit().await?;
        info!(
            "Appended {} resumed transcript segment(s) to meeting {} (audio offset {:.3}s)",
            transcripts.len(),
            meeting_id,
            audio_offset_seconds
        );
        Ok(true)
    }

    /// The meeting's current maximum `audio_end_time` in seconds (0.0 when it has
    /// no timed transcripts yet). A resume path can use this as the audio offset
    /// when it lacks the precise prior audio duration; the audio layer prefers the
    /// concatenated-WAV duration (specs/0037) for exact alignment.
    pub async fn max_audio_end_time(pool: &SqlitePool, meeting_id: &str) -> Result<f64, SqlxError> {
        let v: Option<f64> =
            sqlx::query_scalar("SELECT MAX(audio_end_time) FROM transcripts WHERE meeting_id = ?")
                .bind(meeting_id)
                .fetch_one(pool)
                .await?;
        Ok(v.unwrap_or(0.0))
    }

    /// Loads every transcript segment for a meeting and concatenates them into a
    /// single ordered string, suitable as the input for note enhancement /
    /// summarization. Loading server-side means the enhancement flow does not
    /// depend on the frontend having paginated through all segments.
    ///
    /// Segments are ordered by `audio_start_time` (seconds from recording start)
    /// and then by `timestamp` as a tiebreaker. `audio_start_time` is nullable
    /// (added in a later migration; rows predating it are NULL), so untimed
    /// segments are pushed to the end and ordered by `timestamp` instead of
    /// being sorted first (SQLite's default NULLS FIRST behaviour).
    ///
    /// Returns an empty string when the meeting has no transcript segments.
    pub async fn get_full_transcript(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<String, SqlxError> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT transcript
             FROM transcripts
             WHERE meeting_id = ?
             ORDER BY audio_start_time IS NULL, audio_start_time, timestamp",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;

        let full_transcript = rows
            .into_iter()
            .map(|(transcript,)| transcript.trim().to_string())
            .filter(|transcript| !transcript.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        info!(
            "Loaded full transcript for meeting {} ({} chars)",
            meeting_id,
            full_transcript.len()
        );

        Ok(full_transcript)
    }

    /// Loads ordered transcript segments with their resolved speaker display name
    /// for the notes-aware summary (specs/0010 P2 Task 9). LEFT JOINs `speakers`
    /// so diarized meetings carry `(display_name, text)` while un-diarized meetings
    /// get `(None, text)` for every row (identical to [`Self::get_full_transcript`]).
    ///
    /// Returns rows in the same recording-relative order as `get_full_transcript`.
    /// The caller decides whether to use these (only when at least one segment
    /// resolved a speaker) — keeping the no-diarization summary path byte-identical.
    pub async fn get_transcript_segments_with_speakers(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<(Option<String>, String)>, SqlxError> {
        let rows = sqlx::query_as::<_, (Option<String>, String)>(
            "SELECT s.display_name AS speaker_name, t.transcript
             FROM transcripts t
             LEFT JOIN speakers s
               ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker
             WHERE t.meeting_id = ?
             ORDER BY t.audio_start_time IS NULL, t.audio_start_time, t.timestamp",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }
}

/// specs/0037 — the resume append path: a second session's segments attach to the
/// SAME meeting (no WS6.7 refusal) with their audio times shifted onto the
/// meeting's continuous timeline.
#[cfg(test)]
mod resume_append_tests {
    use super::TranscriptsRepository;
    use crate::database::repositories::meeting::MeetingsRepository;
    use crate::transcripts::TranscriptSegment;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    fn seg(id: &str, text: &str, start: f64, end: f64) -> TranscriptSegment {
        TranscriptSegment {
            id: id.into(),
            text: text.into(),
            timestamp: "2026-07-05T10:00:00Z".into(),
            audio_start_time: Some(start),
            audio_end_time: Some(end),
            duration: Some(end - start),
            channel: Some("microphone".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn append_attaches_a_second_session_with_offset_audio_times() {
        let pool = test_pool().await;
        let meeting_id = MeetingsRepository::create_meeting(
            &pool,
            Some("Standup".into()),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create_meeting");

        // Session 0: 0–2s, 2–4s. Uses the normal (guarded) attach path.
        let attached = TranscriptsRepository::save_transcripts_for_meeting(
            &pool,
            &meeting_id,
            "Standup",
            &[seg("s0a", "hello", 0.0, 2.0), seg("s0b", "world", 2.0, 4.0)],
            Some("/rec/standup".into()),
        )
        .await
        .expect("save session 0");
        assert!(attached, "first session attaches to the empty row");
        assert_eq!(
            TranscriptsRepository::max_audio_end_time(&pool, &meeting_id)
                .await
                .unwrap(),
            4.0
        );

        // Session 1 restarts its clock at 0 (0–3s, 3–5s). Resume offsets by the
        // prior audio duration (4.0s) so the combined timeline stays continuous.
        let offset = 4.0;
        let appended = TranscriptsRepository::append_transcripts_for_meeting(
            &pool,
            &meeting_id,
            &[seg("s1a", "again", 0.0, 3.0), seg("s1b", "bye", 3.0, 5.0)],
            None,
            offset,
        )
        .await
        .expect("append session 1");
        assert!(
            appended,
            "resume appends to the SAME meeting (no WS6.7 refusal)"
        );

        // All four segments live under one meeting, ordered on a continuous timeline.
        let rows: Vec<(String, Option<f64>, Option<f64>)> = sqlx::query_as(
            "SELECT transcript, audio_start_time, audio_end_time FROM transcripts \
             WHERE meeting_id = ? ORDER BY audio_start_time",
        )
        .bind(&meeting_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        let starts: Vec<f64> = rows.iter().map(|r| r.1.unwrap()).collect();
        assert_eq!(
            starts,
            vec![0.0, 2.0, 4.0, 7.0],
            "session-1 times shifted by +4s"
        );
        assert_eq!(
            TranscriptsRepository::max_audio_end_time(&pool, &meeting_id)
                .await
                .unwrap(),
            9.0,
            "combined meeting now ends at 9s (5s session shifted by 4s)"
        );
    }

    #[tokio::test]
    async fn append_is_idempotent_on_a_double_fire() {
        // specs/0037: a resumed stop can double-fire; the second append (same offset)
        // must be a no-op, not a duplicate insert.
        let pool = test_pool().await;
        let meeting_id = MeetingsRepository::create_meeting(
            &pool,
            Some("Standup".into()),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create_meeting");
        TranscriptsRepository::save_transcripts_for_meeting(
            &pool,
            &meeting_id,
            "Standup",
            &[seg("s0a", "hello", 0.0, 2.0), seg("s0b", "world", 2.0, 4.0)],
            None,
        )
        .await
        .expect("save session 0");

        let batch = [seg("s1a", "again", 0.0, 3.0)];
        let first = TranscriptsRepository::append_transcripts_for_meeting(
            &pool,
            &meeting_id,
            &batch,
            None,
            4.0,
        )
        .await
        .expect("first append");
        let second = TranscriptsRepository::append_transcripts_for_meeting(
            &pool,
            &meeting_id,
            &batch,
            None,
            4.0,
        )
        .await
        .expect("second append");
        assert!(first && second, "both calls report success");

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
                .bind(&meeting_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            count, 3,
            "2 prior + 1 appended once (the double-fire is a no-op)"
        );
    }

    #[tokio::test]
    async fn append_to_a_missing_meeting_returns_false() {
        let pool = test_pool().await;
        let appended = TranscriptsRepository::append_transcripts_for_meeting(
            &pool,
            "meeting-does-not-exist",
            &[seg("x", "orphan", 0.0, 1.0)],
            None,
            0.0,
        )
        .await
        .expect("append call");
        assert!(
            !appended,
            "appending to a non-existent meeting is a no-op false"
        );
    }
}
