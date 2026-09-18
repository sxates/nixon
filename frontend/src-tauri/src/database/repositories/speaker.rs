//! `speakers` table access for speaker diarization (specs/0010, ADR-0005, P1-B2).
//!
//! The `speakers` table maps a per-meeting speaker KEY (the value in
//! `transcripts.speaker`: `local` for the mic/local user, `spk_0`,`spk_1`,… for
//! clustered remote speakers) to an editable DISPLAY NAME ("You","Speaker 1", or a
//! user rename like "Priya"). The diarization pipeline upserts one row per detected
//! speaker and stamps every transcript segment's `speaker` key; the transcript read
//! LEFT JOINs back to resolve the display name (see `repositories/meeting.rs`).
//!
//! P1 needs upsert + get + set-segment-keys; full rename/merge is P2 — `rename`
//! is a thin stub here so the command surface is stable.

use crate::database::models::SpeakerModel;
use chrono::Utc;
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use std::collections::HashMap;
use uuid::Uuid;

/// A previously-identified speaker row that carries a stored voiceprint — the
/// candidate "prior art" the cross-meeting matcher (specs/0016 1a, `diarization::
/// identity`) ranks a new meeting's voices against. "Identified" means the row has
/// **both** a non-NULL `embedding` **and** an identity key: an `email`, OR a
/// user-assigned `display_name` that isn't a default ("Speaker N" / "You").
///
/// Carries the raw embedding bytes + `embedding_model` so the matcher can compare
/// only same-model vectors (cosine across models is meaningless — ADR-0007 §4).
#[derive(Debug, Clone, FromRow)]
pub struct IdentifiedSpeaker {
    pub meeting_id: String,
    pub speaker_key: String,
    pub display_name: String,
    pub email: Option<String>,
    /// The durable person this speaker was linked to (specs/0016 1b), when assigned via
    /// `PeopleRepository::assign_speaker_to_person`. The matcher groups candidates by
    /// `person_id` first (strongest cross-meeting signal) before falling back to email/name.
    pub person_id: Option<String>,
    pub embedding: Vec<u8>,
    pub embedding_model: Option<String>,
}

/// A pre-clear snapshot of one speaker row's user-visible identity (WS3.2,
/// specs/0029): everything needed to re-apply a manual rename / attendee assignment
/// after a re-diarization wipes and rebuilds the `speakers` table. `embedding` +
/// `embedding_model` let the pipeline fall back to centroid-similarity matching when
/// the cluster key itself doesn't survive the re-run.
#[derive(Debug, Clone, FromRow)]
pub struct SpeakerIdentitySnapshot {
    pub speaker_key: String,
    pub display_name: String,
    pub email: Option<String>,
    pub person_id: Option<String>,
    pub is_local: i64,
    pub embedding: Option<Vec<u8>>,
    pub embedding_model: Option<String>,
}

pub struct SpeakersRepository;

impl SpeakersRepository {
    /// Insert-or-update a speaker for `(meeting_id, speaker_key)`.
    ///
    /// On conflict with the `UNIQUE(meeting_id, speaker_key)` constraint we refresh
    /// `display_name`/`is_local`/`updated_at` but keep the original `id`/`created_at`.
    /// Idempotent across re-runs.
    ///
    /// `embedding`/`embedding_dim`/`embedding_model` carry the per-remote-speaker
    /// voiceprint (specs/0016 1a). When all three are `Some`, they are written and —
    /// on conflict — refreshed so a re-diarization replaces a stale vector. When `None`
    /// (e.g. the `local`/"You" row, or any non-diarization caller), the existing stored
    /// embedding is **preserved** via COALESCE rather than NULLed out, so passing `None`
    /// on a re-run never strands a previously persisted embedding.
    // The three embedding args are an intentional, spec'd part of the contract
    // (specs/0016 1a) rather than a struct — the column set is small and stable.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
        display_name: &str,
        is_local: bool,
        embedding: Option<&[u8]>,
        embedding_dim: Option<i64>,
        embedding_model: Option<&str>,
    ) -> Result<(), SqlxError> {
        let id = format!("speaker-{}", Uuid::new_v4());
        let now = Utc::now();

        // On INSERT, bind whatever was passed (NULL when None). On CONFLICT, only
        // overwrite when the new value is non-NULL (`COALESCE(excluded.x, speakers.x)`),
        // so a None-arg re-run preserves the prior embedding instead of clearing it.
        sqlx::query(
            "INSERT INTO speakers
                (id, meeting_id, speaker_key, display_name, is_local,
                 embedding, embedding_dim, embedding_model, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(meeting_id, speaker_key) DO UPDATE SET
                display_name    = excluded.display_name,
                is_local        = excluded.is_local,
                embedding       = COALESCE(excluded.embedding, speakers.embedding),
                embedding_dim   = COALESCE(excluded.embedding_dim, speakers.embedding_dim),
                embedding_model = COALESCE(excluded.embedding_model, speakers.embedding_model),
                updated_at      = excluded.updated_at",
        )
        .bind(&id)
        .bind(meeting_id)
        .bind(speaker_key)
        .bind(display_name)
        .bind(is_local as i64)
        .bind(embedding)
        .bind(embedding_dim)
        .bind(embedding_model)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Mint a brand-new MANUAL speaker for a meeting (specs/0039 WS2): a `manual_<uuid>`
    /// key + a `speakers` row with the given display name and a **NULL embedding**.
    /// Returns the new key.
    ///
    /// Used when the user reassigns a mis-clustered span to a speaker the diarizer never
    /// produced (the "New speaker…" affordance). Because the row has no voiceprint, a
    /// later re-diarization cannot re-derive this key from audio — it survives ONLY as
    /// long as a `transcript_speaker_overrides` row references it: a WS2 span correction
    /// re-materializes it via the override-key union in `pipeline::persist`
    /// ([`override_keys_for_meeting`]), and its display name is carried across the re-run
    /// by the WS3.2 rename-restore path (`restore_user_identities`). Clearing every
    /// override that points at it drops it (intended sticky-override semantics). It is
    /// inert for voiceprint enrollment by construction (no embedding to enroll).
    ///
    /// [`override_keys_for_meeting`]: crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository::override_keys_for_meeting
    pub async fn create_manual(
        pool: &SqlitePool,
        meeting_id: &str,
        display_name: &str,
    ) -> Result<String, SqlxError> {
        let speaker_key = format!("manual_{}", Uuid::new_v4());
        Self::upsert(
            pool,
            meeting_id,
            &speaker_key,
            display_name,
            false, // a manual remote speaker, never the local mic user
            None,  // NULL embedding — no audio to derive a voiceprint from
            None,
            None,
        )
        .await?;
        Ok(speaker_key)
    }

    /// Every previously-identified speaker that has a stored voiceprint — the
    /// candidate set for the cross-meeting matcher (specs/0016 1a).
    ///
    /// A row qualifies iff it has BOTH a non-NULL `embedding` AND an identity key:
    /// an `email`, a linked `person_id` (specs/0016 1b — the strongest cross-meeting
    /// key), OR a user-assigned `display_name` that is *not* a generated default. The
    /// defaults are `local`'s "You" and the `spk_N` "Speaker {N+1}" labels written by
    /// the pipeline; we exclude `is_local = 1` rows (never an identity to match against)
    /// and any `display_name` matching `Speaker %`.
    ///
    /// Note this returns identified speakers from *all* meetings, including the
    /// meeting currently being diarized; the matcher is responsible for not
    /// suggesting a speaker against itself (it only ranks rows whose `meeting_id`
    /// differs from / `speaker_key` differs from the current one — but in practice
    /// the current meeting's freshly-upserted rows aren't yet identified, so they
    /// don't qualify here anyway).
    pub async fn get_identified_with_embeddings(
        pool: &SqlitePool,
    ) -> Result<Vec<IdentifiedSpeaker>, SqlxError> {
        let rows = sqlx::query_as::<_, IdentifiedSpeaker>(
            "SELECT meeting_id, speaker_key, display_name, email, person_id,
                    embedding, embedding_model
             FROM speakers
             WHERE embedding IS NOT NULL
               AND is_local = 0
               AND (
                    (email IS NOT NULL AND email <> '')
                    OR person_id IS NOT NULL
                    OR display_name NOT LIKE 'Speaker %'
               )",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// All speakers for a meeting, local user first then by `speaker_key`.
    pub async fn get_by_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerModel>, SqlxError> {
        let speakers = sqlx::query_as::<_, SpeakerModel>(
            "SELECT id, meeting_id, speaker_key, display_name, is_local, email, person_id, created_at, updated_at
             FROM speakers
             WHERE meeting_id = ?
             ORDER BY is_local DESC, speaker_key ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(speakers)
    }

    /// The current meeting's REMOTE speakers that have a stored voiceprint, as
    /// `(speaker_key, embedding_bytes, embedding_model)` — the *query* side of the
    /// cross-meeting matcher (specs/0016 1a). Excludes `local`/"You" (mic channel is
    /// never voiceprinted offline) and any row without an embedding.
    pub async fn get_meeting_embeddings(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<(String, Vec<u8>, Option<String>)>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, Vec<u8>, Option<String>)>(
            "SELECT speaker_key, embedding, embedding_model
             FROM speakers
             WHERE meeting_id = ? AND is_local = 0 AND embedding IS NOT NULL",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// The stored voiceprint for ONE speaker row, as
    /// `(is_local, embedding_bytes, embedding_model)` — the input to enroll-on-confirm
    /// (specs/0016 1c). `None` when the speaker row doesn't exist OR has no embedding
    /// (e.g. a `local`/"You" row offline, or a too-short cluster that wasn't embedded);
    /// the caller then enrolls nothing. Returns the raw bytes; decoding/normalization is
    /// the enroll path's job.
    pub async fn get_speaker_embedding(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
    ) -> Result<Option<(bool, Vec<u8>, Option<String>)>, SqlxError> {
        let row = sqlx::query_as::<_, (i64, Option<Vec<u8>>, Option<String>)>(
            "SELECT is_local, embedding, embedding_model
             FROM speakers
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(meeting_id)
        .bind(speaker_key)
        .fetch_optional(pool)
        .await?;

        Ok(match row {
            Some((is_local, Some(bytes), model)) => Some((is_local != 0, bytes, model)),
            // No row, or a row without an embedding → nothing to enroll.
            _ => None,
        })
    }

    /// Set the `speaker` key on a batch of transcript rows by id, within an existing
    /// transaction. Used by the diarization pipeline after alignment to persist one
    /// key per segment. The (id, key) pairs are applied in order.
    pub async fn set_segment_speakers(
        transaction: &mut sqlx::SqliteConnection,
        assignments: &[(String, String)],
    ) -> Result<(), SqlxError> {
        for (transcript_id, speaker_key) in assignments {
            sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                .bind(speaker_key)
                .bind(transcript_id)
                .execute(&mut *transaction)
                .await?;
        }
        Ok(())
    }

    /// Every speaker row for a meeting as an identity snapshot (WS3.2, specs/0029) —
    /// taken by the diarization pipeline BEFORE [`clear_meeting_speakers`] so
    /// user-set display names / attendee assignments can be re-applied after the
    /// re-run's upsert. Filtering "which rows the user actually touched" happens at
    /// the call site (it needs the generated-default name logic).
    ///
    /// [`clear_meeting_speakers`]: Self::clear_meeting_speakers
    pub async fn get_identity_snapshots(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerIdentitySnapshot>, SqlxError> {
        let rows = sqlx::query_as::<_, SpeakerIdentitySnapshot>(
            "SELECT speaker_key, display_name, email, person_id, is_local,
                    embedding, embedding_model
             FROM speakers
             WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Re-apply a snapshotted user identity onto a (freshly re-upserted) speaker row
    /// (WS3.2, specs/0029): set `display_name` and — when the snapshot carried them —
    /// `email` / `person_id` (COALESCE keeps any value the new run already wrote, e.g.
    /// an auto-label's person link, when the snapshot has none). Returns `Ok(false)`
    /// when no row matches `(meeting_id, speaker_key)`.
    pub async fn restore_identity(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
        display_name: &str,
        email: Option<&str>,
        person_id: Option<&str>,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now();
        let res = sqlx::query(
            "UPDATE speakers SET
                display_name = ?,
                email        = COALESCE(?, email),
                person_id    = COALESCE(?, person_id),
                updated_at   = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(display_name)
        .bind(email)
        .bind(person_id)
        .bind(now)
        .bind(meeting_id)
        .bind(speaker_key)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Clear every speaker key on a meeting's transcripts AND delete its `speakers`
    /// rows, within an existing transaction. Makes a (re-)diarization run idempotent:
    /// the pipeline clears prior keys before writing fresh ones so stale labels never
    /// linger. User renames are preserved across the clear via
    /// [`get_identity_snapshots`](Self::get_identity_snapshots) +
    /// [`restore_identity`](Self::restore_identity) (WS3.2, specs/0029).
    pub async fn clear_meeting_speakers(
        transaction: &mut sqlx::SqliteConnection,
        meeting_id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE transcripts SET speaker = NULL WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM speakers WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;
        Ok(())
    }

    /// Rename a speaker's display name (specs/0010 P2). Returns `Ok(false)` when no
    /// matching speaker row exists. Every rendered segment picks up the new name via
    /// the transcript→speakers LEFT JOIN, so this is a single UPDATE (not N rows).
    pub async fn rename(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
        display_name: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now();
        let res = sqlx::query(
            "UPDATE speakers SET display_name = ?, updated_at = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(display_name)
        .bind(now)
        .bind(meeting_id)
        .bind(speaker_key)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Associate a speaker with a real calendar attendee (specs/0010 P2 Task 7): set
    /// both `display_name` and the stable `email` identity key on the row. Returns
    /// `Ok(false)` when no matching speaker row exists.
    pub async fn assign_to_attendee(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
        display_name: &str,
        email: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now();
        let res = sqlx::query(
            "UPDATE speakers SET display_name = ?, email = ?, updated_at = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(display_name)
        .bind(email)
        .bind(now)
        .bind(meeting_id)
        .bind(speaker_key)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Merge speaker `from_key` into `into_key` for a meeting (specs/0010 P2): reassign
    /// every `transcripts.speaker` row from `from_key` to `into_key`, then delete the
    /// now-orphaned `speakers` row for `from_key`. Runs in one transaction so a
    /// partial merge can never leave dangling keys.
    ///
    /// Idempotent / safe: a no-op when `from_key == into_key`; if `into_key` has no
    /// `speakers` row the reassignment still succeeds (the transcripts simply resolve
    /// to NULL display name until labeled). Returns the number of reassigned segments.
    pub async fn merge(
        pool: &SqlitePool,
        meeting_id: &str,
        from_key: &str,
        into_key: &str,
    ) -> Result<u64, SqlxError> {
        if from_key == into_key {
            return Ok(0);
        }

        let mut tx = pool.begin().await?;

        let reassigned =
            sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
                .bind(into_key)
                .bind(meeting_id)
                .bind(from_key)
                .execute(&mut *tx)
                .await?
                .rows_affected();

        sqlx::query("DELETE FROM speakers WHERE meeting_id = ? AND speaker_key = ?")
            .bind(meeting_id)
            .bind(from_key)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(reassigned)
    }

    /// Per-speaker count of transcript rows currently held in a meeting (specs/0061 W4) —
    /// the "is this speaker empty" signal [`crate::diarization::speaker_maintenance::
    /// prune_empty_speakers_inner`] checks before deleting a `speakers` row. A key absent
    /// from the map has zero rows.
    pub async fn segment_counts(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<HashMap<String, i64>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, i64)>(
            "SELECT speaker, COUNT(*) FROM transcripts
             WHERE meeting_id = ? AND speaker IS NOT NULL
             GROUP BY speaker",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Whether this meeting's speaker key has any stored voiceprint sample referencing it
    /// (specs/0061 W4) — the prune safety check: deleting a speaker a voiceprint still
    /// references would orphan biometric provenance, so such a row is never pruned.
    pub async fn has_voiceprint(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
    ) -> Result<bool, SqlxError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM voiceprints WHERE source_meeting_id = ? AND source_speaker_key = ?)",
        )
        .bind(meeting_id)
        .bind(speaker_key)
        .fetch_one(pool)
        .await
    }

    /// Delete one `speakers` row for a meeting. Returns `Ok(false)` when no matching row
    /// existed (idempotent). Callers own the safety checks (not `local`, zero transcript
    /// rows, no voiceprint) — see [`crate::diarization::speaker_maintenance::
    /// prune_empty_speakers_inner`].
    pub async fn delete(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
    ) -> Result<bool, SqlxError> {
        let res = sqlx::query("DELETE FROM speakers WHERE meeting_id = ? AND speaker_key = ?")
            .bind(meeting_id)
            .bind(speaker_key)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// The earliest (by `audio_start_time`) transcript row id a speaker currently holds in
    /// a meeting (specs/0061 W4) — the click-to-filter jump target.
    pub async fn first_segment_id(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar(
            "SELECT id FROM transcripts
             WHERE meeting_id = ? AND speaker = ?
             ORDER BY audio_start_time ASC LIMIT 1",
        )
        .bind(meeting_id)
        .bind(speaker_key)
        .fetch_optional(pool)
        .await
    }
}
