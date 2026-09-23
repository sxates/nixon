//! `recordings-move.json`: which move was in progress. Written once, atomically, before the
//! first folder moves; deleted when the run ends. It is NOT the source of truth: after a
//! crash it only names the folders to reconcile, and the filesystem and the rows decide
//! what each one needs (the recovery table in `exec.rs`).

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::plan::MoveUnit;

/// The journal's file name, in the app-data folder.
pub const JOURNAL_FILE: &str = "recordings-move.json";

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub version: u32,
    pub target: PathBuf,
    pub units: Vec<MoveUnit>,
}

impl Journal {
    pub fn new(target: PathBuf, units: Vec<MoveUnit>) -> Self {
        Self {
            version: VERSION,
            target,
            units,
        }
    }
}

/// The journal path for this profile.
pub fn default_path() -> PathBuf {
    crate::app_paths::app_data_dir().join(JOURNAL_FILE)
}

/// Write `journal` atomically: a temp file, synced, then renamed over `path`.
pub fn write(path: &Path, journal: &Journal) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Couldn't create {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(journal)?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("Couldn't write {}", tmp.display()))?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("Couldn't write {}", path.display()))?;
    Ok(())
}

/// Read the journal. `None` when there is none, or when nothing in it can be recovered.
///
/// A damaged journal is read as far as it goes: every unit entry that still parses is
/// kept, so the meetings it can still name get reconciled. Anything else is left to the
/// gather that follows, which reconciles from the filesystem and the rows alone.
pub fn read(path: &Path) -> Option<Journal> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::error!("recordings move: couldn't read the journal: {e}");
            return None;
        }
    };
    if let Ok(journal) = serde_json::from_slice::<Journal>(&bytes) {
        return Some(journal);
    }
    log::error!("recordings move: the journal is damaged; recovering what it still names");
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let units: Vec<MoveUnit> = value
        .get("units")
        .and_then(|u| u.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|i| serde_json::from_value(i.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let target = value
        .get("target")
        .and_then(|t| t.as_str())
        .map(PathBuf::from)
        .or_else(|| {
            units
                .first()
                .and_then(|u| u.dst.parent().map(Path::to_path_buf))
        })
        .unwrap_or_default();
    if units.is_empty() && target.as_os_str().is_empty() {
        return None;
    }
    Some(Journal::new(target, units))
}

/// Remove the journal (the run is over). A missing journal is fine.
pub fn delete(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("recordings move: couldn't remove the journal: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(name: &str) -> MoveUnit {
        MoveUnit {
            src: PathBuf::from("/old").join(name),
            dst: PathBuf::from("/new").join(name),
            lease_ids: vec![format!("id-{name}")],
            update_ids: vec![format!("id-{name}")],
            title: name.into(),
            bytes: 3,
            same_volume: true,
            elsewhere: false,
        }
    }

    #[test]
    fn a_journal_round_trips_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(JOURNAL_FILE);
        let journal = Journal::new("/new".into(), vec![unit("a"), unit("b")]);
        write(&path, &journal).unwrap();
        assert_eq!(read(&path), Some(journal));
        assert!(!path.with_extension("json.tmp").exists());
        delete(&path);
        assert_eq!(read(&path), None);
        delete(&path); // idempotent
    }

    #[test]
    fn a_damaged_journal_still_names_the_units_that_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(JOURNAL_FILE);
        let good = serde_json::to_value(unit("a")).unwrap();
        let text = serde_json::json!({ "target": "/new", "units": [good, {"src": 7}] });
        std::fs::write(&path, text.to_string()).unwrap();
        let journal = read(&path).unwrap();
        assert_eq!(journal.units, vec![unit("a")]);
        assert_eq!(journal.target, PathBuf::from("/new"));
    }

    #[test]
    fn an_unreadable_journal_is_treated_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(JOURNAL_FILE);
        std::fs::write(&path, b"{\"units\": [").unwrap();
        assert_eq!(read(&path), None);
    }
}
