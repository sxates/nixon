//! Which meeting folders a move takes, and where each one lands. Pure: every filesystem
//! fact comes through [`PlanFs`], so the classification is unit-tested without disks.
//!
//! The move set is built from THIS profile's meeting rows (plus interrupted recordings whose
//! meeting row exists here). A folder no row of this database names is never moved, which
//! is what keeps a debug build away from production recordings even where the two share a
//! recordings folder.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::roots::MoveRoots;

/// Free space a cross-volume move must leave on the target, on top of the bytes it copies.
pub const SPACE_MARGIN_BYTES: u64 = 1 << 30;

/// Prefix of the hidden staging folder a cross-volume copy is written into.
pub const STAGING_PREFIX: &str = ".nixon-moving-";

/// A meeting row, as the planner sees it.
#[derive(Debug, Clone)]
pub struct PlanRow {
    pub meeting_id: String,
    pub title: String,
    pub folder_path: Option<String>,
}

/// An interrupted recording's folder (specs/0037) whose meeting row exists in this database.
#[derive(Debug, Clone)]
pub struct InterruptedFolder {
    pub meeting_id: String,
    pub folder: PathBuf,
}

/// The filesystem facts the planner needs.
pub trait PlanFs {
    /// Canonical form of `path` (lexical when it doesn't exist).
    fn canonical(&self, path: &Path) -> PathBuf;
    /// Does anything (file or folder) exist at `path`?
    fn exists(&self, path: &Path) -> bool;
    /// Does `path` pass the ownership check for `meeting_id`?
    fn is_owned(&self, path: &Path, meeting_id: &str, known_roots: &[PathBuf]) -> bool;
    /// The `meeting_id` in the folder's `metadata.json`.
    fn meeting_id_of(&self, path: &Path) -> Option<String>;
    /// Total size of the files under `path`.
    fn bytes(&self, path: &Path) -> u64;
    /// Are `path` and `target` on the same volume (a rename can move it)?
    fn same_volume(&self, path: &Path, target: &Path) -> bool;
    /// `root`'s direct children, canonical, without `.DS_Store`.
    fn entries(&self, root: &Path) -> Vec<PathBuf>;
    /// Free bytes on `target`'s volume.
    fn free_bytes(&self, target: &Path) -> u64;
}

/// One folder to move, with every meeting that lives in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveUnit {
    pub src: PathBuf,
    pub dst: PathBuf,
    /// Every meeting whose folder this is. The mover holds each one's lease.
    pub lease_ids: Vec<String>,
    /// The rows whose `folder_path` names `src` and must follow the move. An interrupted
    /// recording whose row has no path is moved without a row update.
    pub update_ids: Vec<String>,
    pub title: String,
    pub bytes: u64,
    pub same_volume: bool,
    /// Outside every known recordings root.
    #[serde(default)]
    pub elsewhere: bool,
}

/// What a move would do. Serialized for the confirm dialog; `units` stays in Rust.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePlan {
    pub target: PathBuf,
    /// Distinct meetings that will move.
    pub meetings: usize,
    /// Folders that will move (a continued meeting shares one folder).
    pub folders: usize,
    pub bytes: u64,
    pub same_volume_count: usize,
    pub cross_volume_count: usize,
    pub cross_volume_bytes: u64,
    /// Folders outside every known recordings root (they still move).
    pub elsewhere: usize,
    /// Meetings whose folder doesn't exist.
    pub missing: usize,
    /// Meetings whose stored folder fails the ownership check (left alone).
    pub not_owned: usize,
    /// Meetings in a folder this build must not touch (debug: the production folder).
    pub protected: usize,
    /// Loose entries in this profile's old folders that no meeting refers to (left alone).
    pub unreferenced: usize,
    pub free_bytes: u64,
    pub enough_space: bool,
    #[serde(skip)]
    pub units: Vec<MoveUnit>,
}

impl MovePlan {
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// The staging folder a cross-volume copy of `dst` is written into.
pub fn staging_path(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dst.with_file_name(format!("{STAGING_PREFIX}{name}"))
}

/// Plan moving every meeting of this profile into `roots.target`.
pub fn plan_move(
    rows: &[PlanRow],
    interrupted: &[InterruptedFolder],
    roots: &MoveRoots,
    fs: &dyn PlanFs,
) -> MovePlan {
    let target = &roots.target;
    let mut plan = MovePlan {
        target: target.clone(),
        ..MovePlan::default()
    };

    // Rows grouped by folder: a continued meeting shares its folder with the original.
    let mut referenced: BTreeSet<PathBuf> = BTreeSet::new();
    let mut groups: BTreeMap<PathBuf, Vec<&PlanRow>> = BTreeMap::new();
    for row in rows {
        let Some(raw) = row
            .folder_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        let folder = fs.canonical(Path::new(raw));
        referenced.insert(folder.clone());
        groups.entry(folder).or_default().push(row);
    }

    let mut units: Vec<MoveUnit> = Vec::new();
    for (folder, group) in &groups {
        let n = group.len();
        if folder.starts_with(target) {
            continue; // already in place
        }
        if roots.is_protected(folder) {
            plan.protected += n;
            continue;
        }
        if !fs.exists(folder) {
            plan.missing += n;
            continue;
        }
        if !group
            .iter()
            .any(|r| fs.is_owned(folder, &r.meeting_id, &roots.known))
        {
            plan.not_owned += n;
            continue;
        }
        let ids: Vec<String> = group.iter().map(|r| r.meeting_id.clone()).collect();
        units.push(new_unit(folder.clone(), ids.clone(), ids, &group[0].title));
    }

    for found in interrupted {
        let folder = fs.canonical(&found.folder);
        if folder.starts_with(target) || roots.is_protected(&folder) {
            continue;
        }
        if let Some(unit) = units.iter_mut().find(|u| u.src == folder) {
            if !unit.lease_ids.contains(&found.meeting_id) {
                unit.lease_ids.push(found.meeting_id.clone());
            }
            continue;
        }
        if referenced.contains(&folder) || !fs.is_owned(&folder, &found.meeting_id, &roots.known) {
            continue; // a row names it and it was classified above
        }
        let title = rows
            .iter()
            .find(|r| r.meeting_id == found.meeting_id)
            .map(|r| r.title.as_str())
            .unwrap_or("Interrupted recording");
        units.push(new_unit(
            folder,
            vec![found.meeting_id.clone()],
            Vec::new(),
            title,
        ));
    }

    units.sort_by(|a, b| a.src.cmp(&b.src));
    let mut taken_names: BTreeSet<String> = BTreeSet::new();
    let mut meeting_ids: BTreeSet<&str> = BTreeSet::new();
    for unit in &mut units {
        unit.dst = destination(unit, target, &mut taken_names, fs);
        unit.bytes = fs.bytes(&unit.src);
        unit.same_volume = fs.same_volume(&unit.src, target);
        unit.elsewhere = !roots.known.iter().any(|r| unit.src.starts_with(r));
    }
    for unit in &units {
        meeting_ids.extend(unit.lease_ids.iter().map(String::as_str));
        plan.bytes += unit.bytes;
        if unit.same_volume {
            plan.same_volume_count += 1;
        } else {
            plan.cross_volume_count += 1;
            plan.cross_volume_bytes += unit.bytes;
        }
        if unit.elsewhere {
            plan.elsewhere += 1;
        }
    }
    plan.meetings = meeting_ids.len();
    plan.folders = units.len();

    let interrupted_folders: BTreeSet<PathBuf> = interrupted
        .iter()
        .map(|f| fs.canonical(&f.folder))
        .collect();
    for root in &roots.sources {
        plan.unreferenced += fs
            .entries(root)
            .iter()
            .filter(|e| {
                !referenced.contains(*e)
                    && !interrupted_folders.contains(*e)
                    && !roots.known.contains(e)
            })
            .count();
    }

    plan.free_bytes = fs.free_bytes(target);
    plan.enough_space = plan.cross_volume_bytes == 0
        || plan.free_bytes >= plan.cross_volume_bytes.saturating_add(SPACE_MARGIN_BYTES);
    plan.units = units;
    plan
}

fn new_unit(
    src: PathBuf,
    lease_ids: Vec<String>,
    update_ids: Vec<String>,
    title: &str,
) -> MoveUnit {
    MoveUnit {
        src,
        dst: PathBuf::new(),
        lease_ids,
        update_ids,
        title: title.to_string(),
        bytes: 0,
        same_volume: true,
        elsewhere: false,
    }
}

/// The folder's own name in `target`, or `name (2)`, `name (3)`… on a collision. A folder
/// already there that names the same meeting is this meeting's earlier, interrupted copy,
/// so it is reused and the mover verifies it against the source (recovery table row 3).
fn destination(
    unit: &MoveUnit,
    target: &Path,
    taken: &mut BTreeSet<String>,
    fs: &dyn PlanFs,
) -> PathBuf {
    let base = unit
        .src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Recording".to_string());
    let src_id = fs.meeting_id_of(&unit.src);
    for n in 1u32.. {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base} ({n})")
        };
        if taken.contains(&name) {
            continue;
        }
        let candidate = target.join(&name);
        let free = !fs.exists(&candidate);
        let same_meeting = !free
            && src_id.is_some()
            && fs.meeting_id_of(&candidate) == src_id
            && unit.lease_ids.iter().any(|id| Some(id) == src_id.as_ref());
        if free || same_meeting {
            taken.insert(name);
            return candidate;
        }
    }
    unreachable!("an unbounded counter always finds a free name")
}
