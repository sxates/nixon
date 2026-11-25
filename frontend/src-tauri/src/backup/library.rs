//! The backups in the backup folder, as the settings screen lists them: made by hand
//! (`Nixon Backup …`, kept until deleted) or by the daily schedule (`Nixon Auto Backup …`,
//! the newest few kept). Only files with exactly those names are listed or deletable.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::BackupError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub path: String,
    pub name: String,
    /// Made by the daily schedule (and pruned with it).
    pub auto: bool,
    pub bytes: u64,
    /// RFC 3339, from the file's modification time.
    pub modified_at: String,
}

/// `Some(auto)` for a backup name of ours (a manual one may carry a ` N` collision suffix).
fn backup_kind(name: &str) -> Option<bool> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^Nixon (Auto )?Backup \d{4}-\d{2}-\d{2} \d{4}( \d+)?\.zip$").unwrap()
    });
    re.captures(name).map(|c| c.get(1).is_some())
}

/// Backups in `folder`, newest first. A missing folder lists nothing.
pub fn list(folder: &Path) -> Vec<BackupFile> {
    let Ok(rd) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<(std::time::SystemTime, BackupFile)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let auto = backup_kind(&name)?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let modified = meta.modified().ok()?;
            let at: chrono::DateTime<chrono::Local> = modified.into();
            Some((
                modified,
                BackupFile {
                    path: e.path().to_string_lossy().into_owned(),
                    name,
                    auto,
                    bytes: meta.len(),
                    modified_at: at.to_rfc3339(),
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    out.into_iter().map(|(_, f)| f).collect()
}

/// Delete one listed backup. Refuses anything that is not a backup file directly inside
/// `folder`, so a crafted path can never delete something else.
pub fn delete(folder: &Path, path: &Path) -> Result<(), BackupError> {
    check_listed(folder, path)?;
    std::fs::remove_file(path)?;
    Ok(())
}

/// Select one listed backup in Finder (`open -R`). Same check as [`delete`].
pub fn reveal(folder: &Path, path: &Path) -> Result<(), BackupError> {
    check_listed(folder, path)?;
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map_err(|e| BackupError::Io(format!("couldn't open Finder: {e}")))?;
    Ok(())
}

/// `path` is a backup file (by name) directly inside `folder`.
fn check_listed(folder: &Path, path: &Path) -> Result<(), BackupError> {
    let refuse = || BackupError::Io("That file isn't a backup in the backup folder".into());
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(refuse)?;
    backup_kind(name).ok_or_else(refuse)?;
    let parent: PathBuf = path.parent().ok_or_else(refuse)?.to_path_buf();
    let same_dir = match (std::fs::canonicalize(&parent), std::fs::canonicalize(folder)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same_dir || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(refuse());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_only_backups_newest_first() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path();
        fs::write(f.join("Nixon Backup 2026-11-20 0930.zip"), b"m").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(f.join("Nixon Auto Backup 2026-11-21 0930.zip"), b"aa").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(f.join("Nixon Backup 2026-11-22 0930 2.zip"), b"m").unwrap();
        for other in ["notes.zip", "Nixon Backup 2026-11-22 0930.zip.partial", "Nixon Backup x.zip"] {
            fs::write(f.join(other), b"x").unwrap();
        }
        fs::create_dir(f.join("Nixon Backup 2026-11-23 0930.zip")).unwrap();
        let names: Vec<_> = list(f).into_iter().map(|b| (b.name, b.auto)).collect();
        assert_eq!(
            names,
            [
                ("Nixon Backup 2026-11-22 0930 2.zip".to_string(), false),
                ("Nixon Auto Backup 2026-11-21 0930.zip".to_string(), true),
                ("Nixon Backup 2026-11-20 0930.zip".to_string(), false),
            ]
        );
        assert!(list(&f.join("missing")).is_empty());
    }

    #[test]
    fn delete_removes_a_listed_backup_only() {
        let d = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let f = d.path();
        let b = f.join("Nixon Backup 2026-11-20 0930.zip");
        fs::write(&b, b"m").unwrap();
        let not_ours = f.join("notes.zip");
        fs::write(&not_ours, b"x").unwrap();
        let elsewhere = other.path().join("Nixon Backup 2026-11-20 0930.zip");
        fs::write(&elsewhere, b"m").unwrap();

        assert!(delete(f, &not_ours).is_err());
        assert!(delete(f, &elsewhere).is_err());
        assert!(not_ours.exists() && elsewhere.exists());
        delete(f, &b).unwrap();
        assert!(!b.exists());
    }
}
