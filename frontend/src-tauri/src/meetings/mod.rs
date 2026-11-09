// Meetings domain: view-model DTOs + Tauri commands over the meetings tables
// (specs/0042 WS3 — dispersed from the old api/api.rs grab-bag).

/// Recorded meetings in an arbitrary date range, for the month calendar (specs/0054 W3).
pub mod calendar_range;
pub mod commands;
/// Deleting a meeting and its folder without waiting on speaker identification (specs/0084).
pub mod delete;
pub mod discard;
/// Which transcript segment a `nixon://meeting/<id>?t=<s>` link points at (specs/0087).
pub mod link_target;
pub mod manual_commands;
/// Whether a recording folder's meeting is already saved (transcript recovery, specs/0083).
pub mod recovery;
pub mod view;

pub use commands::*;
pub use discard::*;
pub use view::*;
