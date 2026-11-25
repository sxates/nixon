//! specs/0092 — back up Nixon and restore it on another Mac.
//!
//! - `manifest.rs`: the archive's `manifest.json`, its validation and the build's schema version.
//! - `files.rs`: which settings files and meeting-folder files travel.
//! - `archive.rs`: the zip writer and the safe extractor.
//! - `merge.rs` / `add.rs`: the Add-what's-missing import (table classification + SQL, orchestration).
//! - `export.rs`: snapshot + settings + meeting folders into one archive.
//! - `progress.rs`: the progress sink trait.
//! - `relocate.rs`: moving staged recording folders into the recordings root.
//! - `replace.rs` / `replace_apply.rs`: Replace restore, staged now and applied at launch.
//! - `settings.rs` / `schedule.rs`: `backup.json` and the daily scheduled backup.
//! - `snapshot.rs`: the consistent DB copy and the logical fingerprint.

pub mod add;
pub mod archive;
pub mod commands;
pub mod export;
pub mod files;
pub mod job;
pub mod manifest;
pub mod merge;
pub mod progress;
pub mod relocate;
pub mod replace;
mod replace_apply;
pub mod schedule;
pub mod settings;
pub mod snapshot;

pub use snapshot::MACHINE_BOUND_TABLES;

use std::fmt;

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Everything a backup, preview or restore can fail with. Serializes as
/// `{ "kind": "<camelCase variant>", "message": "<display text>" }` so the UI maps kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupError {
    NotABackup,
    UnsupportedFormat(u32),
    NewerSchema {
        backup: i64,
        build: i64,
        app_version: String,
    },
    Busy,
    Cancelled,
    Io(String),
    Db(String),
    Zip(String),
    UnsafeEntry(String),
}

impl BackupError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotABackup => "notABackup",
            Self::UnsupportedFormat(_) => "unsupportedFormat",
            Self::NewerSchema { .. } => "newerSchema",
            Self::Busy => "busy",
            Self::Cancelled => "cancelled",
            Self::Io(_) => "io",
            Self::Db(_) => "db",
            Self::Zip(_) => "zip",
            Self::UnsafeEntry(_) => "unsafeEntry",
        }
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABackup => write!(f, "This file is not a Nixon backup"),
            Self::UnsupportedFormat(v) => write!(f, "Unsupported backup format {v}"),
            Self::NewerSchema {
                backup,
                build,
                app_version,
            } => write!(
                f,
                "This backup was made by a newer Nixon ({app_version}, schema {backup}); \
                 this version understands schema {build}. Update Nixon and try again"
            ),
            Self::Busy => write!(f, "A backup or restore is already running"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Io(m) => write!(f, "File error: {m}"),
            Self::Db(m) => write!(f, "Database error: {m}"),
            Self::Zip(m) => write!(f, "Archive error: {m}"),
            Self::UnsafeEntry(m) => write!(f, "Unsafe archive entry: {m}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl Serialize for BackupError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("BackupError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<sqlx::Error> for BackupError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<zip::result::ZipError> for BackupError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Zip(e.to_string())
    }
}
