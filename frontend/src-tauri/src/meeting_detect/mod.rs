// Meeting auto-detection: Zoom (specs/0008 P1), Teams and Google Meet (specs/0074 W6).
//
// A cheap background monitor polls every 3 s for a live call and emits `meeting-detected`
// on a debounced "no call -> call" transition and `meeting-ended` on "call -> no call",
// each carrying the `platform`. The frontend (`MeetingAutoDetect.tsx`) owns the prompt and
// the record start/stop; only a Zoom end stops a recording.
//
// Signals: Zoom's meeting helpers (proven); otherwise a Core Audio client process holding
// the microphone — Teams by bundle id, a browser only with a calendar event in progress or
// (Accessibility already granted) a "Meet - " window title. The Teams/browser signals are
// PROVISIONAL until the owner's spike (`tests/meeting_detect_spike.rs`); see `classify`.
//
// The Zoom mute gate stays in `crate::zoom`, as does the persisted on/off setting
// (`zoom_auto_detect`, key kept for on-disk compatibility).

pub mod calendar;
pub mod classify;
pub mod debounce;
pub mod monitor;
pub mod sample;

pub use monitor::spawn_meeting_monitor;
