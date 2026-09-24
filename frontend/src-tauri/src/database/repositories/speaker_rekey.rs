//! Re-keying a meeting's speaker to and from the owner's `local` key (specs/0078,
//! "This is me" / "This isn't me").
//!
//! "You" is keyed on `local` everywhere downstream (speaker colors, the "You" reassign
//! target, pruning, corrections, the `is_local = 1` gallery filters), so marking a
//! cluster as the owner moves it onto `local` rather than just linking a person to it.
//! Each re-key touches `transcripts.speaker`, `transcript_speaker_overrides`, the
//! `speakers` row and the `voiceprints` source back-links, in ONE transaction: a crash
//! halfway would otherwise leave one voice split across two keys.
//!
//! These live in their own module, as a second `impl SpeakersRepository` block, to keep
//! `speaker.rs` small.

use chrono::Utc;
use sqlx::{Error as SqlxError, SqlitePool};

use super::speaker::SpeakersRepository;

/// The owner's speaker key. Mirrors `diarization::LOCAL_SPEAKER_KEY`, which the repository
/// layer doesn't import.
const LOCAL: &str = "local";

/// A `speakers` row's `(embedding, embedding_dim, embedding_model)`.
type EmbeddingColumns = (Option<Vec<u8>>, Option<i64>, Option<String>);

/// What [`SpeakersRepository::rekey_to_local`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekeyToLocal {
    /// Transcript lines moved from the cluster onto `local`.
    pub moved_lines: u64,
    /// `true` when a `local` row already existed and the cluster was folded into it.
    pub merged_into_existing: bool,
    /// Live samples of OTHER people enrolled from this cluster, now quarantined: the user
    /// just said it is their own voice, so those samples would teach someone else's
    /// gallery the owner's voice. Quarantine is restorable from the People page.
    pub quarantined_other_samples: u64,
}

/// What [`SpeakersRepository::rekey_from_local`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekeyFromLocal {
    /// The `spk_N` key the former "You" lines now carry.
    pub new_key: String,
    /// Transcript lines moved off `local`.
    pub moved_lines: u64,
    /// Live owner samples back-linked to this meeting's `local`, now quarantined.
    pub quarantined_owner_samples: u64,
}

impl SpeakersRepository {
    /// "This is me": move `speaker_key` onto the owner's `local` key for one meeting.
    ///
    /// - `transcripts.speaker` and `transcript_speaker_overrides.speaker_key` are re-pointed.
    /// - Voiceprint samples back-linked to the cluster follow it to `local`; those enrolled
    ///   under anyone other than `owner_person_id` are quarantined first.
    /// - With no `local` row, the cluster's row becomes it: `is_local = 1`, linked to
    ///   `owner_person_id`, named "You", email cleared, embedding kept.
    /// - With a `local` row already there (lines were reassigned to "You" earlier), the
    ///   cluster is folded into it: the row keeps its name, gains the owner link, and takes
    ///   the cluster's embedding only if it has none. The cluster row is deleted.
    ///
    /// `Ok(None)` when the meeting has no `speakers` row for `speaker_key`. The caller
    /// rejects `local` and `unknown` before calling. The owner `people` row is the caller's
    /// to ensure (`people::enroll::ensure_owner_person`).
    pub async fn rekey_to_local(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
        owner_person_id: &str,
    ) -> Result<Option<RekeyToLocal>, SqlxError> {
        if speaker_key == LOCAL {
            return Ok(None);
        }
        let now = Utc::now();
        let mut tx = pool.begin().await?;

        let cluster: Option<EmbeddingColumns> = sqlx::query_as(
            "SELECT embedding, embedding_dim, embedding_model FROM speakers
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(meeting_id)
        .bind(speaker_key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((embedding, embedding_dim, embedding_model)) = cluster else {
            return Ok(None); // dropping `tx` rolls back (nothing was written)
        };
        let local_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM speakers WHERE meeting_id = ? AND speaker_key = ?)",
        )
        .bind(meeting_id)
        .bind(LOCAL)
        .fetch_one(&mut *tx)
        .await?;

        let moved_lines =
            sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
                .bind(LOCAL)
                .bind(meeting_id)
                .bind(speaker_key)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        sqlx::query(
            "UPDATE transcript_speaker_overrides SET speaker_key = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(LOCAL)
        .bind(meeting_id)
        .bind(speaker_key)
        .execute(&mut *tx)
        .await?;

        let quarantined_other_samples = sqlx::query(
            "UPDATE voiceprints SET quarantined_at = ?
             WHERE source_meeting_id = ? AND source_speaker_key = ?
               AND person_id <> ? AND quarantined_at IS NULL",
        )
        .bind(now.to_rfc3339())
        .bind(meeting_id)
        .bind(speaker_key)
        .bind(owner_person_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query(
            "UPDATE voiceprints SET source_speaker_key = ?
             WHERE source_meeting_id = ? AND source_speaker_key = ?",
        )
        .bind(LOCAL)
        .bind(meeting_id)
        .bind(speaker_key)
        .execute(&mut *tx)
        .await?;

        if local_exists {
            // SQLite evaluates every SET expression against the OLD row, so the CASEs all
            // see the pre-update `embedding`: the three columns move together or not at all.
            sqlx::query(
                "UPDATE speakers SET
                    is_local        = 1,
                    person_id       = ?,
                    embedding_dim   = CASE WHEN embedding IS NULL THEN ? ELSE embedding_dim END,
                    embedding_model = CASE WHEN embedding IS NULL THEN ? ELSE embedding_model END,
                    embedding       = COALESCE(embedding, ?),
                    updated_at      = ?
                 WHERE meeting_id = ? AND speaker_key = ?",
            )
            .bind(owner_person_id)
            .bind(embedding_dim)
            .bind(&embedding_model)
            .bind(&embedding)
            .bind(now)
            .bind(meeting_id)
            .bind(LOCAL)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM speakers WHERE meeting_id = ? AND speaker_key = ?")
                .bind(meeting_id)
                .bind(speaker_key)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query(
                "UPDATE speakers SET
                    speaker_key = ?, is_local = 1, person_id = ?, display_name = 'You',
                    email = NULL, updated_at = ?
                 WHERE meeting_id = ? AND speaker_key = ?",
            )
            .bind(LOCAL)
            .bind(owner_person_id)
            .bind(now)
            .bind(meeting_id)
            .bind(speaker_key)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(Some(RekeyToLocal {
            moved_lines,
            merged_into_existing: local_exists,
            quarantined_other_samples,
        }))
    }

    /// "This isn't me": move the meeting's `local` speaker to the next free `spk_N`
    /// ("Speaker N+1"), for a wrong automatic "You".
    ///
    /// Owner samples (`owner_person_id`) back-linked to this meeting's `local` are
    /// quarantined. Their back-link deliberately stays on `local`: it keeps the provenance,
    /// and it keeps the owner bootstrap's one-sample-per-meeting check
    /// (`people::enroll::enroll_owner_sample_from_embedding`) from quietly re-adding a
    /// sample the user just retracted. The row keeps its embedding and loses the owner
    /// link, name and email.
    ///
    /// `Ok(None)` when the meeting has no `local` row.
    pub async fn rekey_from_local(
        pool: &SqlitePool,
        meeting_id: &str,
        owner_person_id: &str,
    ) -> Result<Option<RekeyFromLocal>, SqlxError> {
        let now = Utc::now();
        let mut tx = pool.begin().await?;

        let local_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM speakers WHERE meeting_id = ? AND speaker_key = ?)",
        )
        .bind(meeting_id)
        .bind(LOCAL)
        .fetch_one(&mut *tx)
        .await?;
        if !local_exists {
            return Ok(None);
        }

        // Every `spk_N` key this meeting has ever used, anywhere a key can live, so the new
        // key can't collide with a stale override or a sample's back-link.
        let used: Vec<String> = sqlx::query_scalar(
            "SELECT speaker_key FROM speakers WHERE meeting_id = ?1
             UNION SELECT speaker FROM transcripts WHERE meeting_id = ?1 AND speaker IS NOT NULL
             UNION SELECT speaker_key FROM transcript_speaker_overrides WHERE meeting_id = ?1
             UNION SELECT source_speaker_key FROM voiceprints
                    WHERE source_meeting_id = ?1 AND source_speaker_key IS NOT NULL",
        )
        .bind(meeting_id)
        .fetch_all(&mut *tx)
        .await?;
        let next = next_free_cluster_index(&used);
        let new_key = format!("spk_{next}");

        let quarantined_owner_samples = sqlx::query(
            "UPDATE voiceprints SET quarantined_at = ?
             WHERE source_meeting_id = ? AND source_speaker_key = ?
               AND person_id = ? AND quarantined_at IS NULL",
        )
        .bind(now.to_rfc3339())
        .bind(meeting_id)
        .bind(LOCAL)
        .bind(owner_person_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        let moved_lines =
            sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
                .bind(&new_key)
                .bind(meeting_id)
                .bind(LOCAL)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        sqlx::query(
            "UPDATE transcript_speaker_overrides SET speaker_key = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(&new_key)
        .bind(meeting_id)
        .bind(LOCAL)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE speakers SET
                speaker_key = ?, is_local = 0, person_id = NULL, email = NULL,
                display_name = ?, updated_at = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(&new_key)
        .bind(format!("Speaker {}", next + 1))
        .bind(now)
        .bind(meeting_id)
        .bind(LOCAL)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(RekeyFromLocal {
            new_key,
            moved_lines,
            quarantined_owner_samples,
        }))
    }
}

/// One past the highest `spk_N` index in `keys` (0 when there is none).
fn next_free_cluster_index(keys: &[String]) -> u32 {
    keys.iter()
        .filter_map(|k| k.strip_prefix("spk_")?.parse::<u32>().ok())
        .max()
        .map_or(0, |n| n + 1)
}

#[cfg(test)]
mod tests;
