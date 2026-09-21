// Meetings domain: view-model DTOs + Tauri commands over the meetings tables
// (specs/0042 WS3 — dispersed from the old api/api.rs grab-bag).

/// Recorded meetings in an arbitrary date range, for the month calendar (specs/0054 W3).
pub mod calendar_range;
pub mod commands;
pub mod discard;
pub mod manual_commands;
pub mod view;

pub use commands::*;
pub use discard::*;
pub use view::*;
