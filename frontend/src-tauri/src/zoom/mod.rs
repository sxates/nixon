// Zoom integration: the meeting-detection on/off setting (specs/0008 P1) and the
// self-mute gate (specs/0049).
//
// Detection itself moved to `crate::meeting_detect` (specs/0074 W6), which now covers
// Teams and Google Meet too; its setting keeps the `zoom_auto_detect` key and the
// `api_{get,set}_zoom_auto_detect` commands for on-disk compatibility.

pub mod commands;
/// Zoom self-mute detection via macOS Accessibility (specs/0049).
pub mod mute;
/// Poll that gates the owner mic on Zoom's mute state while recording (specs/0049).
pub mod mute_monitor;
pub mod settings;

pub use mute_monitor::spawn_zoom_mute_monitor;
