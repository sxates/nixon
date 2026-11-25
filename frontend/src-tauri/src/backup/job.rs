//! One backup or restore job at a time (specs/0092): the lock shared by manual export,
//! import, scheduled runs and "Back up now". Also the live status snapshot (for a Settings
//! page that mounts mid-job) and the `backup-progress` / `backup-finished` events.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tokio_util::sync::CancellationToken;

use super::progress::{Phase, Progress};
use super::BackupError;

pub const EVENT_PROGRESS: &str = "backup-progress";
pub const EVENT_FINISHED: &str = "backup-finished";
/// Progress events are throttled to this; the status snapshot is always current.
const EMIT_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Export,
    Import,
    Scheduled,
}

/// What a running job reports; also the `backup-progress` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub kind: JobKind,
    /// `None` until the job reports its first progress.
    pub phase: Option<Phase>,
    pub done: u64,
    pub total: u64,
}

/// The `backup-finished` payload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    pub kind: JobKind,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Finished {
    pub fn ok(kind: JobKind, path: Option<String>) -> Self {
        Self { kind, ok: true, path, error: None }
    }

    pub fn err(kind: JobKind, e: &BackupError) -> Self {
        Self { kind, ok: false, path: None, error: Some(e.to_string()) }
    }
}

struct Slot {
    status: JobStatus,
    token: CancellationToken,
}

static CURRENT: Mutex<Option<Slot>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<Slot>> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Holds the job slot; dropping it frees the slot.
#[derive(Debug)]
pub struct JobGuard {
    token: CancellationToken,
}

impl JobGuard {
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        *lock() = None;
    }
}

/// Claim the one job slot, or `Busy` if any job is running.
pub fn try_begin(kind: JobKind) -> Result<JobGuard, BackupError> {
    let mut slot = lock();
    if slot.is_some() {
        return Err(BackupError::Busy);
    }
    let token = CancellationToken::new();
    *slot = Some(Slot {
        status: JobStatus { kind, phase: None, done: 0, total: 0 },
        token: token.clone(),
    });
    Ok(JobGuard { token })
}

/// Trip the running job's token. False when nothing is running.
pub fn cancel_current() -> bool {
    match lock().as_ref() {
        Some(s) => {
            s.token.cancel();
            true
        }
        None => false,
    }
}

pub fn status() -> Option<JobStatus> {
    lock().as_ref().map(|s| s.status.clone())
}

/// Update the snapshot; returns it, or `None` if no job is running.
fn record(phase: Phase, done: u64, total: u64) -> Option<JobStatus> {
    let mut slot = lock();
    let s = slot.as_mut()?;
    s.status.phase = Some(phase);
    s.status.done = done;
    s.status.total = total;
    Some(s.status.clone())
}

#[cfg(test)]
fn record_progress(phase: Phase, done: u64, total: u64) {
    record(phase, done, total);
}

/// A [`Progress`] that keeps the status snapshot current and emits `backup-progress`.
pub struct EmitProgress<R: Runtime> {
    app: AppHandle<R>,
    last_emit: Mutex<Option<(Instant, Phase)>>,
}

impl<R: Runtime> EmitProgress<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app, last_emit: Mutex::new(None) }
    }
}

impl<R: Runtime> Progress for EmitProgress<R> {
    fn report(&self, phase: Phase, done: u64, total: u64) {
        let Some(status) = record(phase, done, total) else { return };
        let mut last = self.last_emit.lock().unwrap_or_else(|e| e.into_inner());
        let due = match *last {
            Some((at, p)) => p != phase || at.elapsed() >= EMIT_EVERY || (total > 0 && done >= total),
            None => true,
        };
        if due {
            *last = Some((Instant::now(), phase));
            let _ = self.app.emit(EVENT_PROGRESS, &status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The slot is process-wide, so tests that touch it run one at a time.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn second_job_is_busy() {
        let _s = serial();
        let first = try_begin(JobKind::Export).expect("free slot");
        assert_eq!(try_begin(JobKind::Import).err(), Some(BackupError::Busy));
        assert_eq!(try_begin(JobKind::Scheduled).err(), Some(BackupError::Busy));
        drop(first);
    }

    #[test]
    fn guard_drop_frees_slot() {
        let _s = serial();
        let first = try_begin(JobKind::Export).expect("free slot");
        assert_eq!(status().map(|s| s.kind), Some(JobKind::Export));
        drop(first);
        assert!(status().is_none());
        let second = try_begin(JobKind::Import).expect("slot freed");
        drop(second);
    }

    #[test]
    fn cancel_trips_token() {
        let _s = serial();
        assert!(!cancel_current());
        let g = try_begin(JobKind::Import).expect("free slot");
        assert!(!g.token().is_cancelled());
        assert!(cancel_current());
        assert!(g.token().is_cancelled());
    }

    #[test]
    fn status_tracks_progress() {
        let _s = serial();
        let g = try_begin(JobKind::Export).expect("free slot");
        record_progress(Phase::Writing, 5, 10);
        let s = status().expect("running");
        assert_eq!((s.phase, s.done, s.total), (Some(Phase::Writing), 5, 10));
        drop(g);
    }
}
