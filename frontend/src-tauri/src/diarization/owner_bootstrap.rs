//! The owner-voiceprint bootstrap (specs/0078 owner decision 2, W5).
//!
//! Almost no one has an owner voiceprint: before specs/0078 the only path that wrote one
//! was a manual assignment. Without it, a room recording can't find "You" among its
//! clusters. So the diarization pass trains the owner the way it would train anyone,
//! with one head start: a single voice on the mic is assumed to be the owner.
//!
//! - **Call passes:** the bleed-guarded owner turns `inject_owner_turns` derives from the
//!   mic are structurally the owner. When they pool at least
//!   [`OWNER_BOOTSTRAP_MIN_SECS`] of speech, a clip of the longest ones is embedded with
//!   the same [`ClusterEmbedder`] the pass uses for clusters.
//! - **Room passes:** a single cluster is the owner (`room::label_owner_cluster` rule 1);
//!   its cluster embedding is the sample.
//!
//! Either way it is at most one sample per meeting, under the one "Store voiceprints"
//! consent, back-linked to `(meeting, "local")` so "This isn't me" and "Clear all
//! voiceprints" find it (`people::enroll::enroll_owner_sample_from_embedding`). Automatic
//! labels (the owner winning a cluster by voiceprint, or the carry-over) never enroll.
//! Everything here is best-effort: a failure logs and never fails the pass.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::diarization::embedding::{ClusterEmbedder, EMBEDDING_MODEL_ID, MAX_POOL_SECONDS};
use crate::diarization::room::{OwnerLabel, OwnerRule};
use crate::diarization::{SpeakerTurn, LOCAL_SPEAKER_KEY};
use crate::people::enroll::{self, EnrollConfidence, OWNER_PERSON_ID};

/// A call pass bootstraps only when its owner turns pool at least this much speech: a
/// meeting where the owner barely spoke is weak evidence of their voice.
pub const OWNER_BOOTSTRAP_MIN_SECS: f32 = 30.0;

/// The mic audio that becomes the owner sample: the longest owner turns, cut to the
/// embedder's own pool size ([`MAX_POOL_SECONDS`], 8 s). Cluster voiceprints are pooled
/// the same way, so the owner sample is comparable with the clusters it will be matched
/// against, and the cost is one 8 s embedding however long the meeting ran.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnerClip {
    pub samples: Vec<f32>,
    /// Total owner speech in the pass (the ≥ 30 s gate), not the clip's length.
    pub owner_speech_secs: f32,
}

/// Cut the bootstrap clip out of the decoded mic track. `None` below
/// [`OWNER_BOOTSTRAP_MIN_SECS`] of owner speech. Longest turns first: a long
/// uninterrupted stretch is the cleanest owner audio. Pure.
pub(crate) fn owner_clip(
    turns: &[SpeakerTurn],
    mic: &[f32],
    sample_rate: usize,
) -> Option<OwnerClip> {
    let owner_speech_secs: f32 = turns.iter().map(SpeakerTurn::duration).sum();
    if owner_speech_secs < OWNER_BOOTSTRAP_MIN_SECS {
        return None;
    }
    let mut longest_first: Vec<&SpeakerTurn> = turns.iter().collect();
    longest_first.sort_by(|a, b| b.duration().total_cmp(&a.duration()));
    let cap = (MAX_POOL_SECONDS * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(cap);
    for t in longest_first {
        let start = ((t.start.max(0.0) * sample_rate as f32) as usize).min(mic.len());
        let end = ((t.end.max(0.0) * sample_rate as f32) as usize).min(mic.len());
        let take = end.saturating_sub(start).min(cap - samples.len());
        samples.extend_from_slice(&mic[start..start + take]);
        if samples.len() >= cap {
            break;
        }
    }
    (!samples.is_empty()).then_some(OwnerClip {
        samples,
        owner_speech_secs,
    })
}

/// Whether the owner's mic lines were materially reassigned by hand: at least
/// `MATERIAL_CONTEST_FRACTION` of the meeting's mic-tagged rows carry a manual override
/// pointing away from "You" (the specs/0039 WS3 contested guard, applied to the mic).
/// Such a mic track isn't clean owner evidence. Best-effort: a read error counts as
/// contested, so nothing is enrolled on a guess.
async fn mic_rows_contested(pool: &SqlitePool, meeting_id: &str) -> bool {
    use crate::database::repositories::transcript_speaker_overrides::MATERIAL_CONTEST_FRACTION;
    let counts: Result<(i64, i64), _> = sqlx::query_as(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN o.speaker_key IS NOT NULL AND o.speaker_key <> ?
                                  THEN 1 ELSE 0 END), 0)
         FROM transcripts t
         LEFT JOIN transcript_speaker_overrides o ON o.transcript_id = t.id
         WHERE t.meeting_id = ? AND t.channel = 'microphone'",
    )
    .bind(LOCAL_SPEAKER_KEY)
    .bind(meeting_id)
    .fetch_one(pool)
    .await;
    match counts {
        Ok((0, _)) => false,
        Ok((total, moved)) => moved as f64 / total as f64 >= MATERIAL_CONTEST_FRACTION,
        Err(e) => {
            log::warn!("owner bootstrap: couldn't read mic overrides for {meeting_id}: {e}");
            true
        }
    }
}

/// Whether this meeting already gave an owner sample (live or quarantined). The enroll
/// gate checks the same thing; asking first saves loading the embedding model.
async fn has_owner_sample(pool: &SqlitePool, meeting_id: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM voiceprints
                        WHERE person_id = ? AND source_meeting_id = ? AND source_speaker_key = ?)",
    )
    .bind(OWNER_PERSON_ID)
    .bind(meeting_id)
    .bind(LOCAL_SPEAKER_KEY)
    .fetch_one(pool)
    .await
    .unwrap_or(true)
}

/// Embed a call pass's owner clip with the pass's cluster embedder. Blocking (ONNX).
fn embed_clip(clip: OwnerClip) -> anyhow::Result<Option<Vec<f32>>> {
    let paths = crate::diarization::models::model_paths();
    let mut embedder = ClusterEmbedder::new(&paths.embedding)?;
    let turn = SpeakerTurn {
        start: 0.0,
        end: clip.samples.len() as f32 / 16_000.0,
        speaker: LOCAL_SPEAKER_KEY.to_string(),
    };
    Ok(embedder
        .embeddings_for_turns(&[turn], &clip.samples)
        .remove(LOCAL_SPEAKER_KEY))
}

/// After a pass has persisted: enroll the owner bootstrap sample, if this pass has one.
///
/// - Room pass whose owner is the single cluster: that cluster's embedding.
/// - Call pass with an [`OwnerClip`]: the clip's embedding, unless the mic rows are
///   materially contested.
/// - Anything else (an automatic room label, no clip): nothing.
pub async fn enroll_after_pass(
    pool: &SqlitePool,
    meeting_id: &str,
    owner: Option<&OwnerLabel>,
    embeddings: &HashMap<String, Vec<f32>>,
    clip: Option<OwnerClip>,
) {
    let embedding = match (owner, clip) {
        (Some(label), _) if label.rule == OwnerRule::SingleCluster => {
            embeddings.get(LOCAL_SPEAKER_KEY).cloned()
        }
        (Some(_), _) => return, // an automatic label never enrolls
        (None, Some(clip)) => {
            if !enroll::voiceprint_consent().await || has_owner_sample(pool, meeting_id).await {
                return;
            }
            if mic_rows_contested(pool, meeting_id).await {
                log::info!(
                    "owner bootstrap: {meeting_id}'s mic lines are materially reassigned; skipping"
                );
                return;
            }
            let secs = clip.owner_speech_secs;
            match tokio::task::spawn_blocking(move || embed_clip(clip)).await {
                Ok(Ok(v)) => {
                    log::info!(
                        "owner bootstrap: embedded {meeting_id}'s mic owner clip ({secs:.0}s of owner speech)"
                    );
                    v
                }
                Ok(Err(e)) => {
                    log::warn!("owner bootstrap: embedding failed for {meeting_id}: {e:#}");
                    None
                }
                Err(e) => {
                    log::warn!("owner bootstrap: embedding task failed for {meeting_id}: {e}");
                    None
                }
            }
        }
        (None, None) => return,
    };
    let Some(embedding) = embedding else {
        return;
    };
    match enroll::enroll_owner_sample_from_embedding(
        pool,
        meeting_id,
        &embedding,
        EMBEDDING_MODEL_ID,
        EnrollConfidence::OwnerBootstrap,
    )
    .await
    {
        Ok(true) => {
            log::info!("owner bootstrap: added an owner voiceprint sample from {meeting_id}")
        }
        Ok(false) => {}
        Err(e) => log::warn!("owner bootstrap: enrollment failed for {meeting_id}: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f32, end: f32) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker: LOCAL_SPEAKER_KEY.to_string(),
        }
    }

    const SR: usize = 16_000;

    #[test]
    fn under_thirty_seconds_of_owner_speech_gives_no_clip() {
        let mic = vec![0.1f32; 60 * SR];
        let turns = vec![turn(0.0, 10.0), turn(20.0, 39.9)];
        assert_eq!(owner_clip(&turns, &mic, SR), None);
    }

    #[test]
    fn the_clip_is_cut_from_the_longest_turns_and_capped_at_the_pool_size() {
        // Mark each second of the mic with its own value so we can see where samples came from.
        let mic: Vec<f32> = (0..120 * SR).map(|i| (i / SR) as f32).collect();
        // 5 s, then the longest (20 s from t=50), then 10 s.
        let turns = vec![turn(0.0, 5.0), turn(50.0, 70.0), turn(90.0, 100.0)];
        let clip = owner_clip(&turns, &mic, SR).expect("35 s of owner speech");
        assert!((clip.owner_speech_secs - 35.0).abs() < 1e-3);
        assert_eq!(clip.samples.len(), (MAX_POOL_SECONDS as usize) * SR);
        assert_eq!(clip.samples[0], 50.0, "starts in the longest turn");
        assert!(
            clip.samples.iter().all(|&s| (50.0..70.0).contains(&s)),
            "all 8 s fit in the longest turn"
        );
    }

    #[test]
    fn turns_past_the_end_of_the_mic_are_clamped() {
        let mic = vec![0.5f32; 10 * SR];
        let turns = vec![turn(5.0, 40.0)];
        let clip = owner_clip(&turns, &mic, SR).expect("35 s claimed");
        assert_eq!(clip.samples.len(), 5 * SR, "only the 5 s that exist");
    }

    /// The W5 contested guard: a mic track whose lines were mostly moved off "You" by
    /// hand is not clean owner evidence.
    #[tokio::test]
    async fn mic_rows_count_as_contested_once_half_are_moved_off_you() {
        use crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository as Overrides;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let meeting = crate::database::repositories::meeting::MeetingsRepository::create_meeting(
            &pool, None, None, None, None, None,
        )
        .await
        .unwrap();
        for (id, channel) in [("t1", "microphone"), ("t2", "microphone"), ("t3", "system")] {
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, channel, speaker)
                 VALUES (?, ?, 'words', '2026-01-01T00:00:00Z', ?, 'local')",
            )
            .bind(id)
            .bind(&meeting)
            .bind(channel)
            .execute(&pool)
            .await
            .unwrap();
        }
        assert!(!mic_rows_contested(&pool, &meeting).await);
        // Pointing a mic line back at "You" is not a contest.
        Overrides::set(&pool, &meeting, "t1", LOCAL_SPEAKER_KEY)
            .await
            .unwrap();
        assert!(!mic_rows_contested(&pool, &meeting).await);
        // One of two mic lines moved to someone else: 0.5, material.
        Overrides::set(&pool, &meeting, "t2", "spk_0")
            .await
            .unwrap();
        assert!(mic_rows_contested(&pool, &meeting).await);
    }
}
