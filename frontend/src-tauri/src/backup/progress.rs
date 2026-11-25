//! Progress reporting shared by export and restore (specs/0092).

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Snapshot,
    Writing,
    Extracting,
    Applying,
}

/// Receives `(phase, bytes done, bytes total)`. `total` is an estimate that can be 0.
pub trait Progress: Send + Sync {
    fn report(&self, phase: Phase, done: u64, total: u64);
}

pub struct NoProgress;

impl Progress for NoProgress {
    fn report(&self, _phase: Phase, _done: u64, _total: u64) {}
}
