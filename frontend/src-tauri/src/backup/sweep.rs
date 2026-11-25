//! Startup cleanup of large files an interrupted backup or import left behind
//! (specs/0092). Safe only while no job runs and no Replace is pending.

use std::fs;
use std::path::Path;

use super::add::{ADD_RECORDINGS_STAGING_NAME, ADD_STAGING_DIR_NAME};
use super::replace::{MARKER_NAME, RECORDINGS_STAGING_NAME, STAGING_DIR_NAME};

const TEMP_DB_PREFIXES: [&str; 2] = [".backup-snapshot-", ".backup-fingerprint-"];

fn remove_any(p: &Path) {
    if p.is_dir() {
        let _ = fs::remove_dir_all(p);
    } else if p.exists() {
        let _ = fs::remove_file(p);
    }
}

/// Remove leftover snapshot/fingerprint temp DBs and Add/Replace staging in `app_data_dir`,
/// and the staging dirs under each of `recordings_roots`. Does nothing while a
/// `restore-pending.json` marker exists (that staging is about to be applied).
pub fn sweep_leftovers(app_data_dir: &Path, recordings_roots: &[std::path::PathBuf]) {
    if app_data_dir.join(MARKER_NAME).exists() {
        return;
    }
    if let Ok(rd) = fs::read_dir(app_data_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if TEMP_DB_PREFIXES.iter().any(|p| name.starts_with(p)) {
                remove_any(&e.path());
            }
        }
    }
    remove_any(&app_data_dir.join(ADD_STAGING_DIR_NAME));
    remove_any(&app_data_dir.join(STAGING_DIR_NAME));
    for root in recordings_roots {
        remove_any(&root.join(ADD_RECORDINGS_STAGING_NAME));
        remove_any(&root.join(RECORDINGS_STAGING_NAME));
    }
}
