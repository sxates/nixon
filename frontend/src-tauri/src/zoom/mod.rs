// Zoom meeting auto-detection (specs/0008 P1).
//
// A cheap background monitor that detects when a Zoom *meeting* (not just the
// Zoom app) is live on macOS by polling for Zoom's meeting-helper process
// `CptHost` (also `aomhost` / `caphost`), which Zoom spawns only while in a
// meeting and tears down on leave.
//
// On a debounced Idle -> InMeeting transition it emits the Tauri event
// `zoom-meeting-detected`; on InMeeting -> Idle it emits `zoom-meeting-ended`.
// The frontend owns the actual record start/stop — this module only detects,
// emits, and exposes the on/off setting.

pub mod commands;
pub mod monitor;
/// Zoom self-mute detection via macOS Accessibility (specs/0049).
pub mod mute;
/// Poll that gates the owner mic on Zoom's mute state while recording (specs/0049).
pub mod mute_monitor;
pub mod settings;

pub use monitor::spawn_zoom_monitor;
pub use mute_monitor::spawn_zoom_mute_monitor;
