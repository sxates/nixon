//! One folder's move, as a state machine that can be resumed from any crash point.
//!
//! ```text
//! same volume:   rename(src, dst) ─▶ update rows(dst) ─▶ done      (EXDEV ⇒ cross volume)
//! cross volume:  copy src ─▶ staging (synced) ─▶ verify ─▶ rename(staging, dst)
//!                ─▶ update rows(dst) ─▶ remove src ─▶ done
//! ```
//!
//! The rows change only after the destination is complete and verified, and the source is
//! removed only after the rows change. So (does src exist, does dst exist, what do the rows
//! say) always determines the next step: that is [`move_unit`]'s recovery table, and a
//! first run is just its "not started" row. The caller holds every lease of the unit.

use std::path::Path;

use sqlx::SqlitePool;

use super::disk::{copy_synced, is_cross_device, safe_to_remove_source, verify_copy};
use super::plan::{staging_path, MoveUnit};
use super::roots::MoveRoots;
use crate::audio::meeting_folder::canonical_or_lexical;

/// Where a test can make the mover stop dead, as if the app were killed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailPoint {
    /// Cross volume: staging copied, not verified.
    AfterCopy,
    /// Cross volume: staging verified, not renamed.
    AfterVerify,
    /// The destination is in place; the rows still name the source.
    AfterRename,
    /// The rows name the destination; the source is still there.
    AfterRowUpdate,
    /// Part of the source removed.
    DuringSourceDelete,
}

/// Test hooks. Production uses the default (no hooks). These are plain options rather than
/// `#[cfg(test)]` code because the integration tests link the library built without
/// `cfg(test)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecOptions {
    /// Take the copy-verify path even on one volume.
    pub force_cross_volume: bool,
    /// Stop at this point (simulated crash: no cleanup runs).
    pub fail_at: Option<FailPoint>,
    /// Cut one byte off a copied file before the verify (a copy that went wrong).
    pub truncate_copy: bool,
}

/// The run stopped at an injected fail point (tests only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crashed(pub FailPoint);

/// How one folder's move ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitOutcome {
    /// Moved (or a crashed move finished).
    Moved,
    /// Already at the destination; nothing to do.
    AlreadyDone,
    /// Left where it is on purpose (meeting deleted, folder changed meanwhile).
    Skipped(String),
    /// Neither the source nor the destination exists.
    Missing,
    /// Left where it is after an error; the source and rows are untouched.
    Failed(String),
}

fn crash_at(opts: &ExecOptions, point: FailPoint) -> Result<(), Crashed> {
    if opts.fail_at == Some(point) {
        log::warn!("recordings move: injected stop at {point:?}");
        return Err(Crashed(point));
    }
    Ok(())
}

/// What the rows of a unit say, re-read under the lease.
enum Rows {
    /// Every meeting of the unit was deleted meanwhile.
    Gone,
    /// The rows moved somewhere else meanwhile.
    Elsewhere,
    /// At least one row still names the source (these are updated), or the unit has no
    /// row to update (an interrupted recording without a stored path).
    AtSource(Vec<(String, String)>),
    /// The rows name the destination.
    AtDestination,
}

async fn read_rows(pool: &SqlitePool, unit: &MoveUnit) -> Result<Rows, sqlx::Error> {
    let src = canonical_or_lexical(&unit.src);
    let dst = canonical_or_lexical(&unit.dst);
    let (mut present, mut at_src, mut at_dst) = (0usize, Vec::new(), 0usize);
    for id in &unit.lease_ids {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        let Some(path) = row else { continue };
        present += 1;
        let Some(path) = path.filter(|p| !p.trim().is_empty()) else {
            continue;
        };
        let stored = canonical_or_lexical(Path::new(path.trim()));
        if stored == src {
            at_src.push((id.clone(), path));
        } else if stored == dst {
            at_dst += 1;
        }
    }
    Ok(if present == 0 {
        Rows::Gone
    } else if !at_src.is_empty() {
        Rows::AtSource(at_src)
    } else if at_dst > 0 {
        Rows::AtDestination
    } else if unit.update_ids.is_empty() {
        Rows::AtSource(Vec::new())
    } else {
        Rows::Elsewhere
    })
}

/// Point every row that still names the source at the destination, in one transaction.
/// Compare-and-set on the old value, so a row that changed meanwhile is left alone.
async fn update_rows(
    pool: &SqlitePool,
    rows: &[(String, String)],
    dst: &Path,
) -> Result<(), sqlx::Error> {
    let dst = dst.to_string_lossy().to_string();
    let mut tx = pool.begin().await?;
    for (id, old) in rows {
        sqlx::query("UPDATE meetings SET folder_path = ? WHERE id = ? AND folder_path = ?")
            .bind(&dst)
            .bind(id)
            .bind(old)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// Remove the source after the rows point at the destination. A source the guard rejects
/// is left in place (logged); the meeting has moved either way.
fn remove_source(unit: &MoveUnit, roots: &MoveRoots, opts: &ExecOptions) -> Result<(), Crashed> {
    if !safe_to_remove_source(&unit.src, roots) {
        log::error!(
            "recordings move: refusing to remove {} after the move; left in place",
            unit.src.display()
        );
        return Ok(());
    }
    if opts.fail_at == Some(FailPoint::DuringSourceDelete) {
        if let Some(file) = first_file(&unit.src) {
            let _ = std::fs::remove_file(file);
        }
        crash_at(opts, FailPoint::DuringSourceDelete)?;
    }
    if let Err(e) = std::fs::remove_dir_all(&unit.src) {
        log::warn!(
            "recordings move: moved, but couldn't remove the old folder {}: {e}",
            unit.src.display()
        );
    }
    Ok(())
}

fn truncate_one_file(dir: &Path) {
    if let Some(file) = first_file(dir) {
        let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&file) {
            let _ = f.set_len(len.saturating_sub(1));
        }
    }
}

fn first_file(dir: &Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())
}

/// Move one folder, or finish/roll back a move a crash interrupted. The caller holds the
/// lease of every meeting in `unit.lease_ids`. `on_bytes` reports copied bytes.
///
/// Recovery table (src exists, staging exists, dst exists, rows say):
///
/// | src | staging | dst | rows | action |
/// |---|---|---|---|---|
/// | yes | – | no | src | not started: move normally |
/// | yes | yes | no | src | copy interrupted: remove staging, move normally |
/// | yes | no | yes | src | renamed, rows not updated: re-verify, update rows, remove src |
/// | yes | no | yes | dst | rows updated, source kept: remove src |
/// | no | – | yes | src | same-volume rename done: update rows |
/// | no | – | yes | dst | done |
/// | no | – | no | any | never produced by the mover: log, leave the rows, `Missing` |
pub async fn move_unit(
    pool: &SqlitePool,
    unit: &MoveUnit,
    roots: &MoveRoots,
    opts: &ExecOptions,
    on_bytes: &(dyn Fn(u64) + Sync),
) -> Result<UnitOutcome, Crashed> {
    let rows = match read_rows(pool, unit).await {
        Ok(Rows::Gone) => return Ok(UnitOutcome::Skipped("the meeting was deleted".into())),
        Ok(Rows::Elsewhere) => {
            return Ok(UnitOutcome::Skipped(
                "its folder changed while the move waited".into(),
            ))
        }
        Ok(Rows::AtSource(at_src)) => Some(at_src),
        Ok(Rows::AtDestination) => None,
        Err(e) => {
            return Ok(UnitOutcome::Failed(format!(
                "couldn't read the meeting: {e}"
            )))
        }
    };

    let staging = staging_path(&unit.dst);
    if staging.exists() {
        if let Err(e) = std::fs::remove_dir_all(&staging) {
            return Ok(UnitOutcome::Failed(format!(
                "couldn't clear an earlier partial copy: {e}"
            )));
        }
    }

    let (src, dst) = (unit.src.exists(), unit.dst.exists());
    match (src, dst, rows) {
        (true, false, Some(ref at_src)) => {
            fresh_move(pool, unit, at_src, roots, opts, on_bytes).await
        }
        (true, true, Some(ref at_src)) => {
            if let Err(why) = verify_copy(&unit.src, &unit.dst) {
                log::error!(
                    "recordings move: {} doesn't match its original: {why}",
                    unit.dst.display()
                );
                return Ok(UnitOutcome::Failed(format!(
                    "the copy in the new folder doesn't match the original ({why})"
                )));
            }
            if let Err(e) = update_rows(pool, at_src, &unit.dst).await {
                return Ok(UnitOutcome::Failed(format!(
                    "couldn't update the meeting: {e}"
                )));
            }
            crash_at(opts, FailPoint::AfterRowUpdate)?;
            remove_source(unit, roots, opts)?;
            Ok(UnitOutcome::Moved)
        }
        (true, true, None) => {
            remove_source(unit, roots, opts)?;
            Ok(UnitOutcome::Moved)
        }
        (true, false, None) => {
            log::error!(
                "recordings move: {} is stored for the meeting but doesn't exist; left alone",
                unit.dst.display()
            );
            Ok(UnitOutcome::Failed(
                "its folder is missing from the new location".into(),
            ))
        }
        (false, true, Some(ref at_src)) => {
            if let Err(e) = update_rows(pool, at_src, &unit.dst).await {
                return Ok(UnitOutcome::Failed(format!(
                    "couldn't update the meeting: {e}"
                )));
            }
            Ok(UnitOutcome::Moved)
        }
        (false, true, None) => Ok(UnitOutcome::AlreadyDone),
        (false, false, _) => {
            log::error!(
                "recordings move: neither {} nor {} exists; the meeting's row is left alone",
                unit.src.display(),
                unit.dst.display()
            );
            Ok(UnitOutcome::Missing)
        }
    }
}

async fn fresh_move(
    pool: &SqlitePool,
    unit: &MoveUnit,
    at_src: &[(String, String)],
    roots: &MoveRoots,
    opts: &ExecOptions,
    on_bytes: &(dyn Fn(u64) + Sync),
) -> Result<UnitOutcome, Crashed> {
    if let Some(parent) = unit.dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Ok(UnitOutcome::Failed(format!(
                "couldn't create the new folder: {e}"
            )));
        }
    }

    if unit.same_volume && !opts.force_cross_volume {
        match std::fs::rename(&unit.src, &unit.dst) {
            Ok(()) => {
                crash_at(opts, FailPoint::AfterRename)?;
                if let Err(e) = update_rows(pool, at_src, &unit.dst).await {
                    // Put it back so the row and the folder agree again.
                    let _ = std::fs::rename(&unit.dst, &unit.src);
                    return Ok(UnitOutcome::Failed(format!(
                        "couldn't update the meeting: {e}"
                    )));
                }
                crash_at(opts, FailPoint::AfterRowUpdate)?;
                on_bytes(unit.bytes);
                return Ok(UnitOutcome::Moved);
            }
            Err(e) if is_cross_device(&e) => {
                log::info!("recordings move: {} crosses volumes; copying", unit.title);
            }
            Err(e) => {
                return Ok(UnitOutcome::Failed(format!(
                    "couldn't move the folder: {e}"
                )))
            }
        }
    }

    let staging = staging_path(&unit.dst);
    let give_up = |why: String| -> Result<UnitOutcome, Crashed> {
        let _ = std::fs::remove_dir_all(&staging);
        Ok(UnitOutcome::Failed(why))
    };
    if let Err(e) = copy_synced(&unit.src, &staging, on_bytes) {
        return give_up(format!("couldn't copy the recording: {e}"));
    }
    crash_at(opts, FailPoint::AfterCopy)?;
    if opts.truncate_copy {
        truncate_one_file(&staging);
    }
    if let Err(why) = verify_copy(&unit.src, &staging) {
        log::error!(
            "recordings move: copy of {} failed verification: {why}",
            unit.title
        );
        return give_up(format!("the copy didn't match the original ({why})"));
    }
    crash_at(opts, FailPoint::AfterVerify)?;
    if let Err(e) = std::fs::rename(&staging, &unit.dst) {
        return give_up(format!("couldn't finish the copy: {e}"));
    }
    crash_at(opts, FailPoint::AfterRename)?;
    if let Err(e) = update_rows(pool, at_src, &unit.dst).await {
        // The source is intact and the rows still name it: drop the copy.
        let _ = std::fs::remove_dir_all(&unit.dst);
        return Ok(UnitOutcome::Failed(format!(
            "couldn't update the meeting: {e}"
        )));
    }
    crash_at(opts, FailPoint::AfterRowUpdate)?;
    remove_source(unit, roots, opts)?;
    Ok(UnitOutcome::Moved)
}
