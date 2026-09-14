//! Sticky per-segment speaker corrections (specs/0019 WS2.3, note 8).
//!
//! When diarization puts a single transcript line under the wrong speaker, the user can
//! reassign just that line to another speaker. The naive write (`UPDATE transcripts SET
//! speaker = ?`) works until the next offline diarization pass, which
//! `clear_meeting_speakers` (NULLs every `transcripts.speaker`) and rebuilds — silently
//! wiping the fix. So we ALSO record the correction durably here, keyed by the stable
//! `transcripts.id`, and `pipeline::persist` re-applies all of a meeting's overrides at
//! the end of every re-diarization (see `reapply`).
//!
//! One override per line (PK = transcript_id). `set` updates the live transcript AND the
//! override in one transaction so the two never drift; `clear` drops the override (the
//! line reverts to the diarizer's assignment on the next pass).

use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqlitePool};

/// Shared "materially contested" threshold (specs/0039 WS3): a fraction of a cluster's
/// lines at or above this is treated as material. Used by the enroll gate
/// ([`override_fraction_for_speaker`](TranscriptSpeakerOverridesRepository::override_fraction_for_speaker)
/// → refuse enrollment of a heavily hand-edited cluster) and mirrored by the WS3
/// retraction's material-fraction test in `diarization::commands`. 0.5 = "at least half"
/// (owner decision): below it a cluster is still mostly the diarizer's own membership.
pub const MATERIAL_CONTEST_FRACTION: f64 = 0.5;

pub struct TranscriptSpeakerOverridesRepository;

impl TranscriptSpeakerOverridesRepository {
    /// Record (or replace) a manual correction for one transcript line and apply it to
    /// the live transcript immediately, in a single transaction. Returns `Ok(false)`
    /// when no transcript row matches `(meeting_id, transcript_id)` (nothing written).
    pub async fn set(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_id: &str,
        speaker_key: &str,
    ) -> Result<bool, SqlxError> {
        let mut tx = pool.begin().await?;
        // Delegate to the shared bulk writer with a single-line span so the live-update +
        // ON-CONFLICT upsert SQL lives in exactly one place. Applied == 0 means the line
        // didn't belong to this meeting (nothing written).
        let ids = [transcript_id.to_string()];
        let applied =
            Self::apply_corrections_tx(&mut tx, meeting_id, &ids, speaker_key, Utc::now()).await?;
        tx.commit().await?;
        Ok(applied > 0)
    }

    /// Apply a span of per-line speaker corrections within an existing transaction: bulk-
    /// overwrite each line's live `transcripts.speaker` to `speaker_key` AND upsert its
    /// durable override, in two statements total (regardless of span length). Lines that
    /// don't belong to `meeting_id` are skipped — the meeting-guarded `UPDATE ... RETURNING
    /// id` yields exactly the ids that matched, and overrides are written for those and only
    /// those (the foreign-id skip). Returns the number of lines actually reassigned.
    ///
    /// Shared by [`set`](Self::set) and [`set_many`](Self::set_many) so the ON-CONFLICT
    /// upsert SQL exists once.
    async fn apply_corrections_tx(
        conn: &mut sqlx::SqliteConnection,
        meeting_id: &str,
        transcript_ids: &[String],
        speaker_key: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, SqlxError> {
        if transcript_ids.is_empty() {
            return Ok(0);
        }

        // Bulk-apply to the live transcripts, meeting-guarded. RETURNING gives back exactly
        // the ids that belonged to this meeting, so a non-matching (foreign) id is skipped
        // rather than getting a bogus override row.
        let placeholders = std::iter::repeat_n("?", transcript_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let update_sql = format!(
            "UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND id IN ({placeholders}) \
             RETURNING id"
        );
        let mut update_q = sqlx::query_scalar::<_, String>(&update_sql)
            .bind(speaker_key)
            .bind(meeting_id);
        for id in transcript_ids {
            update_q = update_q.bind(id);
        }
        let applied_ids: Vec<String> = update_q.fetch_all(&mut *conn).await?;
        if applied_ids.is_empty() {
            return Ok(0);
        }

        // One multi-row upsert of the durable overrides, for exactly the applied lines.
        let row_placeholders = std::iter::repeat_n("(?, ?, ?, ?)", applied_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let insert_sql = format!(
            "INSERT INTO transcript_speaker_overrides (meeting_id, transcript_id, speaker_key, created_at)
             VALUES {row_placeholders}
             ON CONFLICT(transcript_id) DO UPDATE SET
                 speaker_key = excluded.speaker_key,
                 meeting_id  = excluded.meeting_id"
        );
        let mut insert_q = sqlx::query(&insert_sql);
        for id in &applied_ids {
            insert_q = insert_q
                .bind(meeting_id)
                .bind(id)
                .bind(speaker_key)
                .bind(now);
        }
        insert_q.execute(&mut *conn).await?;

        Ok(applied_ids.len() as u64)
    }

    /// Batch form of [`set`](Self::set) (specs/0039 WS2): record manual corrections for a
    /// SPAN of transcript lines → one target `speaker_key`, applying each to the live
    /// transcript AND recording a durable override, all in a single transaction. Lines
    /// that don't belong to `meeting_id` are skipped (no override written for them).
    /// Returns the number of lines actually reassigned.
    ///
    /// The target `speaker_key` may be a clusterer key (`spk_N`), the `local` mic key,
    /// OR a manually-minted `manual_<uuid>` key with **no embedding**
    /// ([`SpeakersRepository::create_manual`]). Overrides are keyed purely by
    /// `transcript_id → speaker_key`, so the span sticks across a re-diarization via
    /// [`reapply`](Self::reapply) + [`override_keys_for_meeting`](Self::override_keys_for_meeting)
    /// regardless of whether the clusterer can re-derive the key — a `manual_` key it
    /// never can (no audio to cluster from), so such a span persists ONLY through this
    /// table. Clearing the overrides reverts the span to the clusterer's assignment.
    ///
    /// [`SpeakersRepository::create_manual`]: crate::database::repositories::speaker::SpeakersRepository::create_manual
    pub async fn set_many(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_ids: &[String],
        speaker_key: &str,
    ) -> Result<u64, SqlxError> {
        if transcript_ids.is_empty() {
            return Ok(0);
        }

        let mut tx = pool.begin().await?;
        // Two statements total (bulk UPDATE ... RETURNING + one multi-row upsert), regardless
        // of span length, via the shared writer — foreign ids are skipped and the applied
        // count reflects only lines that belonged to this meeting.
        let applied = Self::apply_corrections_tx(
            &mut tx,
            meeting_id,
            transcript_ids,
            speaker_key,
            Utc::now(),
        )
        .await?;
        tx.commit().await?;
        Ok(applied)
    }

    /// Drop a line's override so it reverts to the diarizer's assignment on the next
    /// pass. Does NOT touch the current `transcripts.speaker` (the visible value stays
    /// until re-diarized). Returns whether a row was removed.
    pub async fn clear(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_id: &str,
    ) -> Result<bool, SqlxError> {
        let res = sqlx::query(
            "DELETE FROM transcript_speaker_overrides WHERE transcript_id = ? AND meeting_id = ?",
        )
        .bind(transcript_id)
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Distinct target speaker keys referenced by a meeting's overrides. `persist` unions
    /// these into the set of keys it materializes `speakers` rows for, so a corrected line
    /// pointing at a key the fresh clustering didn't produce still resolves to a name.
    pub async fn override_keys_for_meeting(
        executor: &mut sqlx::SqliteConnection,
        meeting_id: &str,
    ) -> Result<Vec<String>, SqlxError> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT speaker_key FROM transcript_speaker_overrides WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(&mut *executor)
        .await?;
        Ok(rows)
    }

    /// The fraction of `speaker_key`'s CURRENT lines in this meeting that carry a manual
    /// override (specs/0039 WS3) — the "materially contested" signal for the enroll gate.
    ///
    /// - Numerator: override rows whose `transcript_id` is one of the cluster's current
    ///   lines (`transcripts.speaker = speaker_key`).
    /// - Denominator: that cluster's current line count.
    ///
    /// Returns `0.0` when the cluster has no lines (nothing to contest). The enroll gate
    /// refuses only when this reaches [`MATERIAL_CONTEST_FRACTION`], so a single stray
    /// correction on a large clean cluster (a tiny fraction) still enrolls, while a cluster
    /// whose membership was heavily hand-edited (a large fraction) is refused. This replaces
    /// the earlier `has_override_for_speaker`, which over-blocked: ANY single override
    /// refused the whole cluster.
    pub async fn override_fraction_for_speaker(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
    ) -> Result<f64, SqlxError> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcripts WHERE meeting_id = ? AND speaker = ?",
        )
        .bind(meeting_id)
        .bind(speaker_key)
        .fetch_one(pool)
        .await?;
        if total == 0 {
            return Ok(0.0);
        }

        let overridden: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcript_speaker_overrides o
             WHERE o.meeting_id = ?
               AND o.transcript_id IN (
                   SELECT id FROM transcripts WHERE meeting_id = ? AND speaker = ?
               )",
        )
        .bind(meeting_id)
        .bind(meeting_id)
        .bind(speaker_key)
        .fetch_one(pool)
        .await?;

        Ok((overridden as f64) / (total as f64))
    }

    /// Re-apply every override for a meeting onto its transcripts, within an existing
    /// transaction. Called by `pipeline::persist` AFTER the fresh per-segment keys are
    /// written, so manual corrections win over (survive) the re-diarization. Returns the
    /// number of transcript rows updated.
    pub async fn reapply(
        transaction: &mut sqlx::SqliteConnection,
        meeting_id: &str,
    ) -> Result<u64, SqlxError> {
        let res = sqlx::query(
            "UPDATE transcripts
                SET speaker = (
                    SELECT o.speaker_key
                    FROM transcript_speaker_overrides o
                    WHERE o.transcript_id = transcripts.id
                )
              WHERE meeting_id = ?
                AND id IN (
                    SELECT transcript_id FROM transcript_speaker_overrides WHERE meeting_id = ?
                )",
        )
        .bind(meeting_id)
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory pool through the app's real migration set (matches the enroll/voiceprint
    /// repo tests). `foreign_keys` is ON by sqlx default, so a transcript needs its meeting.
    async fn pool_with_schema() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn insert_meeting(pool: &SqlitePool, id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind("t")
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
    }

    /// Insert `n` transcript lines `t{start}..` for a meeting, all under `speaker`.
    async fn insert_lines(pool: &SqlitePool, meeting: &str, start: usize, n: usize, speaker: &str) {
        let now = Utc::now().to_rfc3339();
        for i in start..start + n {
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(format!("t{i}"))
            .bind(meeting)
            .bind("hello")
            .bind(&now)
            .bind(speaker)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    /// A single stray override on a large clean cluster stays well below the material
    /// threshold (so enrollment is NOT refused); overriding half the cluster reaches it.
    #[tokio::test]
    async fn override_fraction_reflects_contested_share() {
        let pool = pool_with_schema().await;
        insert_meeting(&pool, "m1").await;
        // A 4-line cluster spk_1.
        insert_lines(&pool, "m1", 0, 4, "spk_1").await;

        // No overrides → 0.0.
        assert_eq!(
            TranscriptSpeakerOverridesRepository::override_fraction_for_speaker(
                &pool, "m1", "spk_1"
            )
            .await
            .unwrap(),
            0.0
        );

        // One stray correction on a line of the cluster → 1/4 = 0.25 (< MATERIAL).
        TranscriptSpeakerOverridesRepository::set(&pool, "m1", "t0", "spk_1")
            .await
            .unwrap();
        let f1 = TranscriptSpeakerOverridesRepository::override_fraction_for_speaker(
            &pool, "m1", "spk_1",
        )
        .await
        .unwrap();
        assert!(
            f1 < MATERIAL_CONTEST_FRACTION,
            "single stray = {f1}, must be below threshold"
        );

        // A second override → 2/4 = 0.5 (>= MATERIAL) → the cluster is now contested.
        TranscriptSpeakerOverridesRepository::set(&pool, "m1", "t1", "spk_1")
            .await
            .unwrap();
        let f2 = TranscriptSpeakerOverridesRepository::override_fraction_for_speaker(
            &pool, "m1", "spk_1",
        )
        .await
        .unwrap();
        assert!(
            f2 >= MATERIAL_CONTEST_FRACTION,
            "half-overridden = {f2}, must reach threshold"
        );

        // An unknown/empty cluster contests nothing.
        assert_eq!(
            TranscriptSpeakerOverridesRepository::override_fraction_for_speaker(
                &pool, "m1", "nope"
            )
            .await
            .unwrap(),
            0.0
        );
    }

    /// `set_many` reassigns every span line, writes exactly one override per line, and skips
    /// ids that don't belong to the meeting — the applied count excludes the foreign id.
    #[tokio::test]
    async fn set_many_applies_span_and_skips_foreign_ids() {
        let pool = pool_with_schema().await;
        insert_meeting(&pool, "m1").await;
        insert_meeting(&pool, "m2").await;
        insert_lines(&pool, "m1", 0, 3, "spk_1").await; // t0,t1,t2
        insert_lines(&pool, "m2", 9, 1, "spk_9").await; // t9 belongs to m2

        // Reassign t0,t1 (valid) + t9 (foreign) → only 2 applied.
        let ids = vec!["t0".to_string(), "t1".to_string(), "t9".to_string()];
        let applied = TranscriptSpeakerOverridesRepository::set_many(&pool, "m1", &ids, "spk_2")
            .await
            .unwrap();
        assert_eq!(applied, 2, "the foreign id must be skipped");

        // Live transcripts updated for the two valid lines only.
        let moved: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcripts WHERE meeting_id='m1' AND speaker='spk_2'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(moved, 2);
        // t9 untouched (still under its own meeting/speaker).
        let m2_speaker: String =
            sqlx::query_scalar("SELECT speaker FROM transcripts WHERE id='t9'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(m2_speaker, "spk_9");

        // Exactly two override rows written, both for m1, and no row for the foreign id.
        let ovr: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcript_speaker_overrides WHERE meeting_id='m1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ovr, 2);
        let foreign_ovr: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcript_speaker_overrides WHERE transcript_id='t9'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(foreign_ovr, 0, "no override for a foreign id");
    }

    /// `set_many` re-run on the same lines overwrites (not duplicates) the override — the
    /// ON-CONFLICT upsert semantics survive the refactor.
    #[tokio::test]
    async fn set_many_upserts_on_repeat() {
        let pool = pool_with_schema().await;
        insert_meeting(&pool, "m1").await;
        insert_lines(&pool, "m1", 0, 2, "spk_1").await;

        let ids = vec!["t0".to_string(), "t1".to_string()];
        TranscriptSpeakerOverridesRepository::set_many(&pool, "m1", &ids, "spk_2")
            .await
            .unwrap();
        TranscriptSpeakerOverridesRepository::set_many(&pool, "m1", &ids, "spk_3")
            .await
            .unwrap();

        // Still exactly two override rows, now pointing at the latest target.
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT transcript_id, speaker_key FROM transcript_speaker_overrides WHERE meeting_id='m1' ORDER BY transcript_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|(_, k)| k == "spk_3"));
    }
}
