//! specs/0069 W6 — the note the outgoing version leaves for the incoming one.
//!
//! `UpdaterCore` holds the release notes in memory only, so the relaunched binary has no
//! idea it is new, and the user is given no sign that anything happened. Immediately before
//! `app.restart()` the driver writes this receipt; the next launch takes it — once, and only
//! if it names the version actually running — and shows what changed.
//!
//! Version-matched because an install can fail, a user can downgrade, and a Time Machine
//! restore can put an older app beside a newer receipt. Announcing an update that did not
//! happen is worse than announcing nothing.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateReceipt {
    pub version: String,
    pub notes: String,
    pub installed_at: DateTime<Utc>,
}

/// `<app-data-dir>/update_receipt.json` — identifier-derived, like `updater.json` (ADR-0004).
fn receipt_path() -> Result<PathBuf> {
    let path = crate::app_paths::app_data_dir().join("update_receipt.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

pub(crate) fn write_at(path: &Path, receipt: &UpdateReceipt) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(receipt)?)?;
    Ok(())
}

/// Read, delete, and return the receipt only when it names `current_version`.
///
/// The delete is unconditional: a receipt that does not match is stale by definition, and
/// leaving it would either re-announce on every launch or wait to fire after some unrelated
/// future upgrade.
pub(crate) fn take_at(path: &Path, current_version: &str) -> Option<UpdateReceipt> {
    let raw = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    let receipt: UpdateReceipt = match serde_json::from_slice(&raw) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("updater: ignoring an unreadable update receipt: {e}");
            return None;
        }
    };
    if receipt.version != current_version {
        log::info!(
            "updater: discarding a receipt for {} while running {current_version}",
            receipt.version
        );
        return None;
    }
    Some(receipt)
}

/// Best-effort: a failed write costs a "what's new" dialog, never the install.
pub fn write(receipt: &UpdateReceipt) -> Result<()> {
    write_at(&receipt_path()?, receipt)
}

pub fn take(current_version: &str) -> Option<UpdateReceipt> {
    take_at(&receipt_path().ok()?, current_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(version: &str) -> UpdateReceipt {
        UpdateReceipt {
            version: version.to_string(),
            notes: "### Fixed\n\n- A thing".to_string(),
            installed_at: Utc::now(),
        }
    }

    #[test]
    fn a_matching_receipt_is_returned_once_and_then_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_receipt.json");
        write_at(&path, &receipt("0.8.0")).unwrap();

        let taken = take_at(&path, "0.8.0").expect("matching receipt");
        assert_eq!(taken.version, "0.8.0");
        assert_eq!(taken.notes, "### Fixed\n\n- A thing");
        assert!(!path.exists(), "the receipt is consumed, not left to re-show");
        assert!(take_at(&path, "0.8.0").is_none());
    }

    #[test]
    fn a_receipt_for_another_version_is_dropped_silently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_receipt.json");
        write_at(&path, &receipt("0.8.0")).unwrap();

        // The install did not take (or the user downgraded): the running binary is older.
        assert!(take_at(&path, "0.7.0").is_none());
        assert!(!path.exists(), "a stale receipt is deleted, not kept forever");
    }

    #[test]
    fn no_receipt_and_a_corrupt_receipt_both_yield_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_receipt.json");
        assert!(take_at(&path, "0.8.0").is_none());

        std::fs::write(&path, b"{not json").unwrap();
        assert!(take_at(&path, "0.8.0").is_none());
        assert!(!path.exists());
    }
}
