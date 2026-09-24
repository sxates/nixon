//! Room recordings: everyone on one shared mic (specs/0078).
//!
//! Diarization was built for calls: the owner is on the mic, everyone else is on the
//! system track, and only the system track is clustered. An in-person meeting recorded on
//! a laptop or a speakerphone has an effectively silent system track, so it used to come
//! out as one speaker, "You". This module decides, per pass, which setup a meeting was
//! recorded in, and holds the room-only steps the offline pass branches into:
//!
//! 1. [`resolve_diarization_input`]: find the recording folder, measure how active each
//!    channel is ([`ChannelActivity`]), apply the stored override, and pick the track to
//!    cluster (the mic in a room, the system track in a call).
//! 2. [`room_speaker_ceiling`]: the owner is now one of the clusters, so the roster cap
//!    grows by one.
//! 3. [`label_owner_cluster`]: find the owner among the clusters and re-key them to
//!    `local`, before split/align, so every consumer that understands "You" keeps working.
//!
//! Room mode never writes `transcripts.channel`: the tag stays the raw capture record.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::PathBuf;

use anyhow::Result;
use sqlx::SqlitePool;

use crate::audio::channel_writer::{mic_channel_path, system_channel_path, system_channel_wav};
use crate::audio::retranscription_channels::ChannelRmsProfile;
use crate::database::repositories::meeting_audio_setup::MeetingAudioSetupRepository;
use crate::diarization::embedding::{cosine_similarity, embedding_to_bytes, EMBEDDING_MODEL_ID};
use crate::diarization::room_types::{AudioSetup, AudioSetupOverride};
use crate::diarization::{SpeakerCount, SpeakerTurn, LOCAL_SPEAKER_KEY, UNKNOWN_SPEAKER_KEY};

// ---------------------------------------------------------------------------
// Detection
//
// Measured on two real recordings with the capture classifier's 600 ms windows and its
// 0.01 RMS bar (specs/0078 "Measured on the real samples"):
//
// | Track                          | Duration | Active            | Longest run | Runs |
// |--------------------------------|----------|-------------------|-------------|------|
// | In-person sample, system       | 3,259 s  | 4.8 s (0.15%)     | 1.2 s       | 7    |
// | In-person sample, mic          | 3,259 s  | 2,800.8 s (86%)   | 57.6 s      | 443  |
// | Two-channel call sample, system| 3,068 s  | 2,136.6 s (69.6%) | 28.2 s      | 556  |
// | Two-channel call sample, mic   | 3,068 s  | 2,661.6 s (87%)   | 295.8 s     | 344  |
//
// The in-person system blips are isolated 1–2 s notification sounds. The thresholds rest
// on these two recordings plus synthetic fixtures; they are not broadly tuned.
// ---------------------------------------------------------------------------

/// (a) No sustained system sound: the longest run of consecutive active system windows
/// must be shorter than this. Notification sounds peaked at 1.2 s in the in-person sample;
/// remote speech in the call sample ran up to 28.2 s.
pub const ROOM_SYS_MAX_RUN_SECS: f32 = 3.0;

/// (b) Little system sound overall: total active system time must be at most
/// `min(ROOM_SYS_MAX_ACTIVE_SECS, ROOM_SYS_MAX_ACTIVE_FRACTION × duration)`. The in-person
/// sample had 4.8 s over 54 minutes; the call sample had 2,136.6 s. The fraction catches
/// many dings in a short meeting that (a) alone would let through.
pub const ROOM_SYS_MAX_ACTIVE_SECS: f32 = 30.0;

/// (b), relative part: 2% of the meeting's duration.
pub const ROOM_SYS_MAX_ACTIVE_FRACTION: f32 = 0.02;

/// (c) Something to diarize: at least this much active mic time. Below it, the pass stays
/// a call, which keeps a short solo note as "You" exactly as before.
pub const ROOM_MIC_MIN_ACTIVE_SECS: f32 = 30.0;

/// How active each capture channel is, over 600 ms windows at the capture classifier's
/// `CHANNEL_ACTIVE_RMS` bar (built by `ChannelRmsProfile::activity`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelActivity {
    pub duration_secs: f32,
    /// Sum of the active system windows.
    pub system_active_secs: f32,
    /// Longest run of consecutive active system windows.
    pub system_longest_run_secs: f32,
    pub mic_active_secs: f32,
    /// `false` when the meeting has no system channel file at all.
    pub system_present: bool,
}

impl fmt::Display for ChannelActivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "duration {:.1}s, system {} active {:.1}s (longest run {:.1}s), mic active {:.1}s",
            self.duration_secs,
            if self.system_present {
                "present,"
            } else {
                "missing,"
            },
            self.system_active_secs,
            self.system_longest_run_secs,
            self.mic_active_secs
        )
    }
}

/// Whether the system track is effectively silent by conditions (a) and (b). A missing
/// system channel satisfies both.
pub fn system_is_quiet(a: &ChannelActivity) -> bool {
    if !a.system_present {
        return true;
    }
    let active_cap = ROOM_SYS_MAX_ACTIVE_SECS.min(ROOM_SYS_MAX_ACTIVE_FRACTION * a.duration_secs);
    a.system_longest_run_secs < ROOM_SYS_MAX_RUN_SECS && a.system_active_secs <= active_cap
}

/// Whether a meeting is a room recording: a quiet system track (a, b) and enough mic
/// speech to diarize (c).
pub fn detect_room(a: &ChannelActivity) -> bool {
    system_is_quiet(a) && a.mic_active_secs >= ROOM_MIC_MIN_ACTIVE_SECS
}

/// The setup a pass runs with, from the stored override and the measured activity.
///
/// - `Call` override → `Call`.
/// - `Room` override → `Room`. (Once hybrid (W4) ships, a `Room` override over an active
///   system track becomes `Hybrid`; until then it runs as `Room`.)
/// - `Auto` → `Room` when [`detect_room`], else `Call`. Auto never picks `Hybrid`: every
///   speakers-without-headphones call has bleed on the mic.
/// - No activity (no channel files, or undecodable) → `Call`, today's behavior.
pub fn resolve_setup(ovr: AudioSetupOverride, activity: Option<&ChannelActivity>) -> AudioSetup {
    match ovr {
        AudioSetupOverride::Call => AudioSetup::Call,
        AudioSetupOverride::Room => AudioSetup::Room,
        AudioSetupOverride::Auto => {
            if activity.is_some_and(detect_room) {
                AudioSetup::Room
            } else {
                AudioSetup::Call
            }
        }
    }
}

/// Whether a pass's setup came from detection or from the user's override. On the wire
/// as the complete event's `audioSetupSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupSource {
    Detected,
    Override,
}

impl SetupSource {
    pub fn of(ovr: AudioSetupOverride) -> Self {
        match ovr {
            AudioSetupOverride::Auto => SetupSource::Detected,
            _ => SetupSource::Override,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SetupSource::Detected => "detected",
            SetupSource::Override => "override",
        }
    }
}

/// The speaker-count ceiling for a pass. The roster/calendar count excludes the owner,
/// because a call clusters only the remote side; when the owner is among the clusters
/// (room/hybrid) the cap grows by one. `Auto`, `Fixed` and the "no cap" `AtMost(0)` are
/// unchanged, and the 0050 audio seed in the diarizer still applies on top.
pub fn room_speaker_ceiling(count: SpeakerCount, setup: AudioSetup) -> SpeakerCount {
    match count {
        SpeakerCount::AtMost(n) if n > 0 && setup.owner_is_clustered() => {
            SpeakerCount::AtMost(n.saturating_add(1))
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Resolving a pass's input
// ---------------------------------------------------------------------------

/// What one diarization pass runs on.
#[derive(Debug, Clone, PartialEq)]
pub struct DiarizationInput {
    pub folder: PathBuf,
    pub setup: AudioSetup,
    pub source: SetupSource,
    /// The track to cluster: `mic` in a room, `system` in a call. In a call with no system
    /// channel this is the (missing) `system.wav` path, so decoding fails exactly as it
    /// did before specs/0078.
    pub cluster_wav: PathBuf,
    /// `None` when detection didn't run (a `Call` override) or had nothing to read.
    pub activity: Option<ChannelActivity>,
}

/// Pick the setup and the track to cluster for `folder`. A room needs a mic channel; with
/// none, the pass falls back to a call.
pub(crate) fn choose_input(
    folder: PathBuf,
    ovr: AudioSetupOverride,
    activity: Option<ChannelActivity>,
) -> DiarizationInput {
    let mic = mic_channel_path(&folder);
    let mut setup = resolve_setup(ovr, activity.as_ref());
    if setup.owner_is_clustered() && mic.is_none() {
        log::warn!(
            "diarization: {} requested but {} has no mic channel; running as a call",
            setup.as_str(),
            folder.display()
        );
        setup = AudioSetup::Call;
    }
    let cluster_wav = match (setup, mic) {
        (AudioSetup::Room | AudioSetup::Hybrid, Some(mic)) => mic,
        _ => system_channel_path(&folder).unwrap_or_else(|| system_channel_wav(&folder)),
    };
    DiarizationInput {
        folder,
        setup,
        source: SetupSource::of(ovr),
        cluster_wav,
        activity,
    }
}

/// Resolve the recording folder, measure the channels, apply the stored override, record
/// the result in `meetings.audio_setup_resolved`, and log the decision. Called under the
/// pass's folder lease.
///
/// Detection decodes both tracks, one at a time, keeping only per-window RMS; it is
/// skipped under a `Call` override, where it can't change the answer.
pub async fn resolve_diarization_input(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<DiarizationInput> {
    let folder = super::folder_locate::resolve_meeting_folder(pool, meeting_id).await?;
    let ovr = match MeetingAudioSetupRepository::get(pool, meeting_id).await {
        Ok(Some(stored)) => stored.override_setup,
        Ok(None) => AudioSetupOverride::Auto,
        Err(e) => {
            log::warn!(
                "diarization: couldn't read the audio setup for {meeting_id} ({e}); detecting"
            );
            AudioSetupOverride::Auto
        }
    };
    let activity = if ovr == AudioSetupOverride::Call {
        None
    } else {
        let f = folder.clone();
        tokio::task::spawn_blocking(move || {
            ChannelRmsProfile::load_for_detection(&f).map(|p| p.activity())
        })
        .await
        .unwrap_or_else(|e| {
            log::warn!("diarization: channel activity task failed ({e}); treating as a call");
            None
        })
    };
    let input = choose_input(folder, ovr, activity);

    match MeetingAudioSetupRepository::set_resolved(pool, meeting_id, input.setup).await {
        Ok(true) => {}
        Ok(false) => {
            log::warn!("diarization: no meeting row to record the audio setup for {meeting_id}")
        }
        Err(e) => log::warn!("diarization: couldn't record the audio setup for {meeting_id}: {e}"),
    }
    log::info!(
        "diarization audio setup for {meeting_id}: {} ({}, override {}); room detection {}; \
         activity: {}",
        input.setup.as_str(),
        input.source.as_str(),
        ovr.as_str(),
        match input.activity.as_ref() {
            Some(a) if detect_room(a) => "yes",
            Some(_) => "no",
            None => "not run",
        },
        input
            .activity
            .map_or_else(|| "not measured".to_string(), |a| a.to_string())
    );
    Ok(input)
}

// ---------------------------------------------------------------------------
// Finding the owner among the clusters
// ---------------------------------------------------------------------------

/// Which rule picked the owner's cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerRule {
    /// The only voice in the room: the person recording is in it.
    SingleCluster,
    /// The owner's gallery voiceprint won the cluster by the ordinary auto-label rules.
    Voiceprint,
    /// The meeting's previous `local` embedding (a prior pass, or "This is me") matched.
    CarryOver,
}

/// The owner's cluster, as picked by [`choose_owner_cluster`].
#[derive(Debug, Clone, PartialEq)]
pub struct OwnerLabel {
    /// The cluster key before it was re-keyed to `local`.
    pub cluster: String,
    pub rule: OwnerRule,
    /// The cosine behind a voiceprint or carry-over pick.
    pub score: Option<f32>,
}

/// The clustered voices in `turns`: every key but `unknown` (and `local`, which sherpa
/// never produces).
fn cluster_keys(turns: &[SpeakerTurn]) -> BTreeSet<String> {
    turns
        .iter()
        .map(|t| t.speaker.as_str())
        .filter(|k| *k != UNKNOWN_SPEAKER_KEY && *k != LOCAL_SPEAKER_KEY)
        .map(str::to_string)
        .collect()
}

/// The owner-labeling rules, in order (specs/0078, as amended by the owner decisions):
///
/// 1. Exactly one cluster → it is the owner.
/// 2. `owner_voiceprint`: the cluster the owner won as an ordinary gallery candidate
///    (same thresholds and sample floor as anyone; computed by the caller).
/// 3. `prior_local`: this meeting's previous `local` embedding, matched against the
///    clusters at `TAU_MATCH`, the same bar `restore_user_identities` carries renames at.
/// 4. Otherwise nobody: every cluster stays "Speaker N".
///
/// Pure, so the rule order is unit-testable without a DB or a model.
pub(crate) fn choose_owner_cluster(
    turns: &[SpeakerTurn],
    embeddings: &HashMap<String, Vec<f32>>,
    owner_voiceprint: Option<(String, f32)>,
    prior_local: Option<&[f32]>,
) -> Option<OwnerLabel> {
    let keys = cluster_keys(turns);
    if keys.len() == 1 {
        return keys.into_iter().next().map(|cluster| OwnerLabel {
            cluster,
            rule: OwnerRule::SingleCluster,
            score: None,
        });
    }
    if let Some((cluster, score)) = owner_voiceprint.filter(|(k, _)| keys.contains(k)) {
        return Some(OwnerLabel {
            cluster,
            rule: OwnerRule::Voiceprint,
            score: Some(score),
        });
    }
    let prior = prior_local?;
    keys.iter()
        .filter_map(|k| embeddings.get(k).map(|v| (k, cosine_similarity(prior, v))))
        .filter(|(_, sim)| *sim >= crate::diarization::identity::TAU_MATCH)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, sim)| OwnerLabel {
            cluster: k.clone(),
            rule: OwnerRule::CarryOver,
            score: Some(sim),
        })
}

/// Re-key one cluster to `local` in the pass's in-memory result: its turns and its
/// embedding. Everything downstream (split, align, persist) then sees "You".
pub(crate) fn rename_cluster_to_local(
    turns: &mut [SpeakerTurn],
    embeddings: &mut HashMap<String, Vec<f32>>,
    cluster: &str,
) {
    for t in turns.iter_mut().filter(|t| t.speaker == cluster) {
        t.speaker = LOCAL_SPEAKER_KEY.to_string();
    }
    if let Some(v) = embeddings.remove(cluster) {
        embeddings.insert(LOCAL_SPEAKER_KEY.to_string(), v);
    }
}

/// The cluster the owner wins as an ordinary gallery candidate, if any. The whole
/// candidate set competes (the owner's margin over other people matters), and only a
/// match the matcher marks `auto_label` counts. Best-effort: a read failure means none.
async fn owner_voiceprint_match(
    pool: &SqlitePool,
    meeting_id: &str,
    keys: &BTreeSet<String>,
    embeddings: &HashMap<String, Vec<f32>>,
    corroborating_emails: &[String],
) -> Option<(String, f32)> {
    use crate::diarization::candidates::{keep_owner_only_as_auto_label, load_candidates};
    use crate::people::enroll::OWNER_PERSON_ID;

    let candidates = match load_candidates(pool, meeting_id, true).await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("diarization: owner match skipped for {meeting_id}: {e:#}");
            return None;
        }
    };
    let current: Vec<(String, Vec<u8>, Option<String>)> = keys
        .iter()
        .filter_map(|k| {
            embeddings.get(k).map(|v| {
                (
                    k.clone(),
                    embedding_to_bytes(v),
                    Some(EMBEDDING_MODEL_ID.to_string()),
                )
            })
        })
        .collect();
    let suggestions = crate::diarization::identity::match_speakers_with_gallery(
        &current,
        &candidates,
        corroborating_emails,
    );
    keep_owner_only_as_auto_label(suggestions)
        .into_iter()
        .find(|s| s.suggested_person_id.as_deref() == Some(OWNER_PERSON_ID))
        .map(|s| (s.speaker_key, s.confidence))
}

/// This meeting's current `local` embedding (from a prior room pass or "This is me"),
/// snapshotted before `persist` clears the rows. Only a same-model vector counts.
async fn prior_local_embedding(pool: &SqlitePool, meeting_id: &str) -> Option<Vec<f32>> {
    use crate::database::repositories::speaker::SpeakersRepository;
    let (_, bytes, model) =
        SpeakersRepository::get_speaker_embedding(pool, meeting_id, LOCAL_SPEAKER_KEY)
            .await
            .ok()??;
    if model.as_deref() != Some(EMBEDDING_MODEL_ID) {
        return None;
    }
    crate::diarization::embedding::embedding_from_bytes(&bytes).ok()
}

/// Find the owner among a room pass's clusters and re-key that cluster to `local`, in
/// memory, before split/align/persist (see [`choose_owner_cluster`] for the rules). An
/// automatic label never enrolls; the single-cluster bootstrap enrollment happens after
/// persist (`owner_bootstrap`).
pub async fn label_owner_cluster(
    pool: &SqlitePool,
    meeting_id: &str,
    turns: &mut [SpeakerTurn],
    embeddings: &mut HashMap<String, Vec<f32>>,
    corroborating_emails: &[String],
) -> Option<OwnerLabel> {
    let keys = cluster_keys(turns);
    let (voiceprint, prior) = if keys.len() > 1 {
        (
            owner_voiceprint_match(pool, meeting_id, &keys, embeddings, corroborating_emails).await,
            prior_local_embedding(pool, meeting_id).await,
        )
    } else {
        (None, None)
    };
    let label = choose_owner_cluster(turns, embeddings, voiceprint, prior.as_deref());
    match &label {
        Some(l) => {
            rename_cluster_to_local(turns, embeddings, &l.cluster);
            log::info!(
                "diarization: room pass for {meeting_id}: {} is the owner ({:?}{}) among {} cluster(s)",
                l.cluster,
                l.rule,
                l.score.map_or(String::new(), |s| format!(", cosine {s:.3}")),
                keys.len()
            );
        }
        None => log::info!(
            "diarization: room pass for {meeting_id}: owner not identified among {} cluster(s); \
             all stay \"Speaker N\"",
            keys.len()
        ),
    }
    label
}

#[cfg(test)]
mod tests;
