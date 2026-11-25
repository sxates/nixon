//! `manifest.json` at the root of a backup archive (specs/0092, ADR-0021).

use serde::{Deserialize, Serialize};

use super::BackupError;

/// The archive layout version this build writes and reads.
pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub meetings: i64,
    pub people: i64,
    pub topics: i64,
    pub action_items: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingRef {
    pub meeting_id: String,
    pub folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub app: String,
    pub app_version: String,
    pub schema_version: i64,
    pub created_at: String,
    pub includes_audio: bool,
    pub counts: Counts,
    pub recordings: Vec<RecordingRef>,
}

/// The newest migration version this build ships (what a freshly migrated DB reports).
pub fn build_schema_version() -> i64 {
    sqlx::migrate!("./migrations")
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0)
}

/// Reject archives this build cannot restore: an unknown format, or a database newer than
/// this build's schema. Older schemas are fine (they get migrated on restore).
pub fn validate(m: &Manifest, build_schema: i64) -> Result<(), BackupError> {
    if m.format != FORMAT {
        return Err(BackupError::UnsupportedFormat(m.format));
    }
    if m.schema_version > build_schema {
        return Err(BackupError::NewerSchema {
            backup: m.schema_version,
            build: build_schema,
            app_version: m.app_version.clone(),
        });
    }
    Ok(())
}
