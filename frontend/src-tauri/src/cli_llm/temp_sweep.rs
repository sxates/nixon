//! Startup clean-up of run dirs a crash left behind (specs/0085).
//!
//! The runner removes its `nixon-cli-XXXX` dir when a call returns, but a crash or a force
//! quit mid-call leaves it (with the system prompt inside) in the temp dir. At launch,
//! [`sweep`] removes those that are ours and old. A call lasts at most five minutes, so a
//! dir untouched for [`STALE_AFTER`] can't belong to a run in flight.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// The runner's temp-dir prefix.
pub const PREFIX: &str = "nixon-cli-";
/// Older than the longest call (300 s) plus its kill grace, with room to spare.
pub const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Removes `nixon-cli-*` dirs directly in `root` that the current user owns and that were
/// last modified more than `older_than` ago. Best-effort: every error is skipped. Returns
/// how many were removed.
pub fn sweep(root: &Path, older_than: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(PREFIX) {
            continue;
        }
        // DirEntry metadata does not follow symlinks, so a link is never a dir here.
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() || !owned_by_me(&meta) {
            continue;
        }
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > older_than);
        if stale && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(unix)]
fn owned_by_me(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions and cannot fail.
    meta.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn owned_by_me(_meta: &std::fs::Metadata) -> bool {
    true
}

/// Runs [`sweep`] on the system temp dir on a blocking thread. Call once at startup.
pub fn spawn_startup_sweep() {
    tauri::async_runtime::spawn_blocking(|| {
        let removed = sweep(&std::env::temp_dir(), STALE_AFTER);
        if removed > 0 {
            log::info!("cli_llm: removed {removed} leftover temp dir(s) from an earlier run");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn age(path: &Path, by: Duration) {
        File::open(path)
            .unwrap()
            .set_modified(SystemTime::now() - by)
            .unwrap();
    }

    #[test]
    fn removes_only_old_nixon_cli_dirs() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let old = r.join("nixon-cli-old111");
        std::fs::create_dir_all(old.join("in")).unwrap();
        std::fs::write(old.join("in/system.txt"), "synthetic").unwrap();
        age(&old, Duration::from_secs(3600));

        let fresh = r.join("nixon-cli-fresh22");
        std::fs::create_dir(&fresh).unwrap();

        let other = r.join("someone-else-old");
        std::fs::create_dir(&other).unwrap();
        age(&other, Duration::from_secs(3600));

        let file = r.join("nixon-cli-afile");
        std::fs::write(&file, "x").unwrap();
        age(&file, Duration::from_secs(3600));

        // A symlink named like ours is never followed or removed.
        let target = tempfile::tempdir().unwrap();
        let link = r.join("nixon-cli-link");
        std::os::unix::fs::symlink(target.path(), &link).unwrap();

        assert_eq!(sweep(r, STALE_AFTER), 1);
        assert!(!old.exists());
        assert!(fresh.exists(), "a dir in use (recent) is kept");
        assert!(other.exists(), "not ours by name");
        assert!(file.exists(), "only dirs");
        assert!(link.symlink_metadata().is_ok() && target.path().exists());
    }

    #[test]
    fn a_missing_root_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(sweep(&root.path().join("gone"), STALE_AFTER), 0);
    }
}
