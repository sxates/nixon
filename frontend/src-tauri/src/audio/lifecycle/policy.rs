//! The audio retention policy (specs/0072): one pure function decides keep / compress /
//! delete for a meeting. The post-processing hook, the startup + hourly sweep, the
//! "apply now" after a settings change and the dry-run preview all call [`disposition`],
//! so they can't disagree.
//!
//! No I/O here: the caller gathers [`MeetingAudioFacts`] (DB row + folder scan).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Under "Once processed", audio whose speaker identification failed is kept this many
/// days so the user can retry, then deleted anyway (owner decision Q1).
pub const FAILED_AUDIO_GRACE_DAYS: u32 = 7;

/// How long kept audio lives. Stored in `RecordingPreferences::audio_retention`
/// as `{"mode":"after_processing"} | {"mode":"days","days":N} | {"mode":"forever"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AudioRetention {
    /// "Once processed": delete once the meeting is transcribed and speakers identified.
    AfterProcessing,
    /// Delete audio from meetings strictly older than `days`; keep (compressed) until then.
    Days { days: u32 },
    /// Keep (compressed) forever.
    Forever,
}

impl AudioRetention {
    /// The one-time derivation from the pre-0072 pair: `auto_save=false` was the
    /// "Immediately" choice whatever `retention_days` said; otherwise a day count or forever.
    pub fn from_legacy(auto_save: bool, retention_days: Option<u32>) -> Self {
        if !auto_save {
            return Self::AfterProcessing;
        }
        match retention_days {
            Some(days) if days > 0 => Self::Days { days },
            _ => Self::Forever,
        }
    }

    /// The legacy pair this policy maps onto, for code (and UI) that still reads it.
    /// "Once processed" keeps the stored day count so switching back restores it.
    pub fn to_legacy(self, stored_days: Option<u32>) -> (bool, Option<u32>) {
        match self {
            Self::AfterProcessing => (false, stored_days),
            Self::Days { days } => (true, Some(days)),
            Self::Forever => (true, None),
        }
    }
}

/// Where a meeting's audio is in its life (`meetings.audio_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioState {
    /// NULL in the database: processing hasn't finished.
    Pending,
    Processed,
    /// Speaker identification errored; audio kept for a retry.
    Failed,
    /// The retention policy deleted the audio.
    Purged,
}

impl AudioState {
    pub fn from_db(value: Option<&str>) -> Self {
        match value {
            Some("processed") => Self::Processed,
            Some("failed") => Self::Failed,
            Some("purged") => Self::Purged,
            _ => Self::Pending,
        }
    }

    /// The column value (`None` = NULL).
    pub fn as_db(self) -> Option<&'static str> {
        match self {
            Self::Pending => None,
            Self::Processed => Some("processed"),
            Self::Failed => Some("failed"),
            Self::Purged => Some("purged"),
        }
    }

    pub fn as_str(self) -> &'static str {
        self.as_db().unwrap_or("pending")
    }
}

/// Everything [`disposition`] needs to know about one meeting.
#[derive(Debug, Clone)]
pub struct MeetingAudioFacts {
    pub state: AudioState,
    /// `meetings.created_at`; age is measured from it, as before 0072.
    pub created_at: DateTime<Utc>,
    /// The folder's `metadata.json` says `status: "recording"`: the live session or a
    /// crash-interrupted one awaiting the resume prompt.
    pub folder_recording: bool,
    /// The meeting still awaits transcription (the backlog predicate, or a `'live'`
    /// handoff marker).
    pub awaiting_transcription: bool,
    /// Channel files are still uncompressed WAVs.
    pub has_wav_channels: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    Recording,
    AwaitingProcessing,
    Processing,
    Failed,
    /// The policy keeps it and there's nothing to compress.
    Policy,
    AlreadyPurged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Keep(KeepReason),
    /// Processed, the policy keeps it, and its channels are still WAV.
    Compress,
    Delete,
}

/// Strictly older than `days` (a meeting exactly `days` old is kept).
fn older_than(created_at: DateTime<Utc>, days: u32, now: DateTime<Utc>) -> bool {
    created_at < now - Duration::days(i64::from(days))
}

/// The single source of truth. See the spec's table (§"The policy function").
pub fn disposition(
    policy: AudioRetention,
    m: &MeetingAudioFacts,
    now: DateTime<Utc>,
) -> Disposition {
    use Disposition::{Compress, Delete, Keep};
    if m.state == AudioState::Purged {
        return Keep(KeepReason::AlreadyPurged);
    }
    if m.folder_recording {
        return Keep(KeepReason::Recording);
    }
    if m.awaiting_transcription {
        return Keep(KeepReason::AwaitingProcessing);
    }
    let compress_or_keep = || {
        if m.has_wav_channels {
            Compress
        } else {
            Keep(KeepReason::Policy)
        }
    };
    match (m.state, policy) {
        (AudioState::Pending, _) => Keep(KeepReason::Processing),
        (AudioState::Processed, AudioRetention::AfterProcessing) => Delete,
        (AudioState::Processed, AudioRetention::Days { days }) => {
            if older_than(m.created_at, days, now) {
                Delete
            } else {
                compress_or_keep()
            }
        }
        (AudioState::Processed, AudioRetention::Forever) => compress_or_keep(),
        (AudioState::Failed, AudioRetention::AfterProcessing) => {
            if older_than(m.created_at, FAILED_AUDIO_GRACE_DAYS, now) {
                Delete
            } else {
                Keep(KeepReason::Failed)
            }
        }
        (AudioState::Failed, AudioRetention::Days { days }) => {
            if older_than(m.created_at, days, now) {
                Delete
            } else {
                Keep(KeepReason::Failed)
            }
        }
        (AudioState::Failed, AudioRetention::Forever) => Keep(KeepReason::Failed),
        (AudioState::Purged, _) => Keep(KeepReason::AlreadyPurged),
    }
}

/// Which meeting-level retention setting wins when a preferences object is saved: the
/// explicit `audio_retention` when the sender changed it, else the legacy pair when *that*
/// changed (an older settings screen), else whatever was stored. Pure so the rule is tested.
pub fn reconcile_saved_retention(
    stored: Option<AudioRetention>,
    stored_legacy: (bool, Option<u32>),
    incoming: Option<AudioRetention>,
    incoming_legacy: (bool, Option<u32>),
) -> AudioRetention {
    let stored_effective =
        stored.unwrap_or_else(|| AudioRetention::from_legacy(stored_legacy.0, stored_legacy.1));
    match incoming {
        Some(policy) if Some(policy) != stored => policy,
        _ if incoming_legacy != stored_legacy => {
            AudioRetention::from_legacy(incoming_legacy.0, incoming_legacy.1)
        }
        _ => stored_effective,
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
