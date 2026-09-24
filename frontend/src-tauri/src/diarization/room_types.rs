//! The audio-setup vocabulary for room recordings (specs/0078).
//!
//! Two enums, kept apart from the detection logic (`diarization/room.rs`) so the
//! database layer and the IPC commands can name them without pulling in audio code:
//!
//! - [`AudioSetupOverride`] is what the user stored: detect automatically, or force
//!   "everyone in the room" / "just me on the mic".
//! - [`AudioSetup`] is what one diarization pass actually used.
//!
//! Both round-trip through lowercase strings, which are the DB values
//! (`meetings.audio_setup`, `meetings.audio_setup_resolved`) and the wire values.

/// The per-meeting "Who was on the mic?" override (`meetings.audio_setup`).
///
/// `Auto` is stored as NULL, so a meeting nobody touched reads as `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioSetupOverride {
    /// Detect from the audio (the default).
    #[default]
    Auto,
    /// Everyone was in the room, on one shared mic.
    Room,
    /// Only the owner was on the mic; everyone else was on the call.
    Call,
}

impl AudioSetupOverride {
    /// The wire string: `"auto" | "room" | "call"`.
    pub fn as_str(self) -> &'static str {
        match self {
            AudioSetupOverride::Auto => "auto",
            AudioSetupOverride::Room => "room",
            AudioSetupOverride::Call => "call",
        }
    }

    /// Parse a wire string. `None` for anything that isn't one of the three values, so
    /// a command can reject a bad argument with a clear message.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(AudioSetupOverride::Auto),
            "room" => Some(AudioSetupOverride::Room),
            "call" => Some(AudioSetupOverride::Call),
            _ => None,
        }
    }

    /// The `meetings.audio_setup` column value. `Auto` is NULL.
    pub fn to_db(self) -> Option<&'static str> {
        match self {
            AudioSetupOverride::Auto => None,
            other => Some(other.as_str()),
        }
    }

    /// Read the `meetings.audio_setup` column. NULL, and any value this build doesn't
    /// know, read as `Auto`: an unreadable override must fall back to detection rather
    /// than fail the pass.
    pub fn from_db(value: Option<&str>) -> Self {
        match value.and_then(Self::parse) {
            Some(v) => v,
            None => {
                if let Some(raw) = value {
                    log::warn!("unknown meetings.audio_setup value {raw:?}; treating as auto");
                }
                AudioSetupOverride::Auto
            }
        }
    }
}

/// The setup one diarization pass ran with (`meetings.audio_setup_resolved`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSetup {
    /// The owner on the mic, everyone else on the system track (today's behavior).
    Call,
    /// Everyone on the mic; the system track is effectively silent.
    Room,
    /// People on both tracks (specs/0078 W4; override-only).
    Hybrid,
}

impl AudioSetup {
    /// The wire and DB string: `"call" | "room" | "hybrid"`.
    pub fn as_str(self) -> &'static str {
        match self {
            AudioSetup::Call => "call",
            AudioSetup::Room => "room",
            AudioSetup::Hybrid => "hybrid",
        }
    }

    /// Parse a wire/DB string; `None` for anything unknown.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "call" => Some(AudioSetup::Call),
            "room" => Some(AudioSetup::Room),
            "hybrid" => Some(AudioSetup::Hybrid),
            _ => None,
        }
    }

    /// Whether the owner is found among the clusters in this setup (room/hybrid), rather
    /// than being the mic channel by construction (call).
    pub fn owner_is_clustered(self) -> bool {
        matches!(self, AudioSetup::Room | AudioSetup::Hybrid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_round_trips_through_wire_and_db() {
        for v in [
            AudioSetupOverride::Auto,
            AudioSetupOverride::Room,
            AudioSetupOverride::Call,
        ] {
            assert_eq!(AudioSetupOverride::parse(v.as_str()), Some(v));
            assert_eq!(AudioSetupOverride::from_db(v.to_db()), v);
        }
        assert_eq!(
            AudioSetupOverride::Auto.to_db(),
            None,
            "auto is stored as NULL"
        );
        assert_eq!(
            AudioSetupOverride::parse(" Room "),
            Some(AudioSetupOverride::Room)
        );
        assert_eq!(
            AudioSetupOverride::parse("hybrid"),
            None,
            "not an override value"
        );
        assert_eq!(
            AudioSetupOverride::from_db(Some("garbage")),
            AudioSetupOverride::Auto
        );
    }

    #[test]
    fn setup_round_trips_and_knows_where_the_owner_is() {
        for v in [AudioSetup::Call, AudioSetup::Room, AudioSetup::Hybrid] {
            assert_eq!(AudioSetup::parse(v.as_str()), Some(v));
        }
        assert_eq!(AudioSetup::parse("auto"), None);
        assert!(!AudioSetup::Call.owner_is_clustered());
        assert!(AudioSetup::Room.owner_is_clustered());
        assert!(AudioSetup::Hybrid.owner_is_clustered());
    }
}
