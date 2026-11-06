//! specs/0083 W1: keep the Mac awake while a meeting is recording.
//!
//! On battery, macOS idle-sleeps a Mac nobody is touching, and a sleeping Mac drops the
//! meeting's audio. While recording we hold two things:
//! - an IOKit `PreventUserIdleSystemSleep` assertion (the system stays awake; the display may
//!   still sleep), visible by name in `pmset -g assertions`;
//! - an `NSProcessInfo` activity (`NSActivityUserInitiatedAllowingIdleSystemSleep`) that turns
//!   App Nap off without itself blocking idle sleep.
//!
//! [`hold`] returns an RAII [`KeepAwake`] guard. A process-wide [`HoldRegistry`] refcounts
//! live guards per resource, so the OS objects are created on the first hold and released
//! on the last. The registry takes a [`KeepAwakeBackend`], so the counting is unit-tested
//! with a fake. A backend failure is logged and never fails a recording.
//!
//! The recording lifecycle uses the one-slot helpers [`begin_recording`] /
//! [`end_recording`]: start acquires after `IS_RECORDING` flips, and stop releases it the
//! moment capture ends (owner decision 2026-11-05: sleeping during processing is fine).

use std::sync::{LazyLock, Mutex, MutexGuard};

use crate::app_paths::APP_NAME;

/// Why a hold is taken. `Recording` holds both resources; `WrapUp` holds only the App Nap
/// activity (which still allows idle sleep), so it can never keep the Mac awake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    Recording,
    /// specs/0083 W2: the background wrap-up after stop. Unused until W2 lands.
    #[cfg_attr(not(test), allow(dead_code))]
    WrapUp,
}

impl HoldReason {
    fn holds_sleep_assertion(self) -> bool {
        matches!(self, HoldReason::Recording)
    }
}

/// The OS side of a hold. Implementations keep their own handles; every call must be safe to
/// make when the matching create/begin failed earlier.
pub(crate) trait KeepAwakeBackend: Send {
    fn create_sleep_assertion(&mut self, name: &str) -> Result<(), String>;
    fn release_sleep_assertion(&mut self);
    fn begin_activity(&mut self, reason: &str) -> Result<(), String>;
    fn end_activity(&mut self);
}

fn sleep_assertion_name() -> String {
    format!("{APP_NAME} is recording a meeting")
}

/// Refcounts live holds per resource and drives the backend on the 0 → 1 and 1 → 0 edges.
pub(crate) struct HoldRegistry {
    backend: Box<dyn KeepAwakeBackend>,
    sleep_holds: usize,
    activity_holds: usize,
}

impl HoldRegistry {
    pub(crate) fn new(backend: Box<dyn KeepAwakeBackend>) -> Self {
        Self {
            backend,
            sleep_holds: 0,
            activity_holds: 0,
        }
    }

    fn acquire(&mut self, reason: HoldReason) {
        if reason.holds_sleep_assertion() {
            self.sleep_holds += 1;
            if self.sleep_holds == 1 {
                match self.backend.create_sleep_assertion(&sleep_assertion_name()) {
                    Ok(()) => log::info!("keep-awake: sleep assertion acquired ({reason:?})"),
                    Err(e) => log::warn!(
                        "keep-awake: could not create the sleep assertion ({e}); the Mac may idle-sleep during this recording"
                    ),
                }
            }
        }
        self.activity_holds += 1;
        if self.activity_holds == 1 {
            match self.backend.begin_activity(&sleep_assertion_name()) {
                Ok(()) => log::info!("keep-awake: App Nap activity begun ({reason:?})"),
                Err(e) => log::warn!("keep-awake: could not begin the App Nap activity ({e})"),
            }
        }
    }

    fn release(&mut self, reason: HoldReason) {
        if reason.holds_sleep_assertion() && self.sleep_holds > 0 {
            self.sleep_holds -= 1;
            if self.sleep_holds == 0 {
                self.backend.release_sleep_assertion();
                log::info!("keep-awake: sleep assertion released ({reason:?})");
            }
        }
        if self.activity_holds > 0 {
            self.activity_holds -= 1;
            if self.activity_holds == 0 {
                self.backend.end_activity();
                log::info!("keep-awake: App Nap activity ended ({reason:?})");
            }
        }
    }
}

/// No-op backend for platforms without a keep-awake implementation.
#[cfg(not(target_os = "macos"))]
struct NoopBackend;

#[cfg(not(target_os = "macos"))]
impl KeepAwakeBackend for NoopBackend {
    fn create_sleep_assertion(&mut self, _name: &str) -> Result<(), String> {
        Ok(())
    }
    fn release_sleep_assertion(&mut self) {}
    fn begin_activity(&mut self, _reason: &str) -> Result<(), String> {
        Ok(())
    }
    fn end_activity(&mut self) {}
}

fn platform_backend() -> Box<dyn KeepAwakeBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(super::keep_awake_macos::MacBackend::default())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NoopBackend)
    }
}

static REGISTRY: LazyLock<Mutex<HoldRegistry>> =
    LazyLock::new(|| Mutex::new(HoldRegistry::new(platform_backend())));

/// A poisoned lock must never panic a recording start or stop.
fn lock(registry: &Mutex<HoldRegistry>) -> MutexGuard<'_, HoldRegistry> {
    registry.lock().unwrap_or_else(|e| e.into_inner())
}

/// RAII hold. Dropping it releases the hold.
#[must_use = "dropping the guard releases the hold immediately"]
pub struct KeepAwake {
    registry: &'static Mutex<HoldRegistry>,
    reason: HoldReason,
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        lock(self.registry).release(self.reason);
    }
}

fn hold_in(registry: &'static Mutex<HoldRegistry>, reason: HoldReason) -> KeepAwake {
    lock(registry).acquire(reason);
    KeepAwake { registry, reason }
}

/// Take a hold on the process-wide registry.
pub fn hold(reason: HoldReason) -> KeepAwake {
    hold_in(&REGISTRY, reason)
}

/// The current recording's hold. One slot: a second [`begin_recording`] is a no-op.
static RECORDING_HOLD: Mutex<Option<KeepAwake>> = Mutex::new(None);

fn lock_slot(slot: &Mutex<Option<KeepAwake>>) -> MutexGuard<'_, Option<KeepAwake>> {
    slot.lock().unwrap_or_else(|e| e.into_inner())
}

fn begin_in(slot: &Mutex<Option<KeepAwake>>, registry: &'static Mutex<HoldRegistry>) {
    let mut slot = lock_slot(slot);
    if slot.is_none() {
        *slot = Some(hold_in(registry, HoldReason::Recording));
    }
}

fn end_in(slot: &Mutex<Option<KeepAwake>>) {
    // Take it out first so the guard drops (taking the registry lock) after the slot unlocks.
    let taken = lock_slot(slot).take();
    drop(taken);
}

/// Recording start (after `IS_RECORDING` flips): keep the Mac awake until [`end_recording`].
pub fn begin_recording() {
    begin_in(&RECORDING_HOLD, &REGISTRY);
}

/// Recording stop or failed start: release the recording's hold. Idempotent.
pub fn end_recording() {
    end_in(&RECORDING_HOLD);
}

/// Calls [`end_recording`] when dropped, so every return path of `stop_recording` (including
/// early errors) releases the hold. Drop it explicitly at the point capture has stopped.
pub struct EndRecordingOnDrop;

impl Drop for EndRecordingOnDrop {
    fn drop(&mut self) {
        end_recording();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    type Calls = Arc<Mutex<Vec<String>>>;

    struct FakeBackend {
        calls: Calls,
        fail: bool,
    }

    impl KeepAwakeBackend for FakeBackend {
        fn create_sleep_assertion(&mut self, name: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("create:{name}"));
            if self.fail {
                Err("IOReturn 0xe00002c2".into())
            } else {
                Ok(())
            }
        }
        fn release_sleep_assertion(&mut self) {
            self.calls.lock().unwrap().push("release".into());
        }
        fn begin_activity(&mut self, _reason: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push("begin".into());
            if self.fail {
                Err("no process info".into())
            } else {
                Ok(())
            }
        }
        fn end_activity(&mut self) {
            self.calls.lock().unwrap().push("end".into());
        }
    }

    fn fake_registry(fail: bool) -> (&'static Mutex<HoldRegistry>, Calls) {
        let calls: Calls = Arc::default();
        let backend = FakeBackend {
            calls: calls.clone(),
            fail,
        };
        let registry = Box::leak(Box::new(Mutex::new(HoldRegistry::new(Box::new(backend)))));
        (registry, calls)
    }

    fn take(calls: &Calls) -> Vec<String> {
        std::mem::take(&mut *calls.lock().unwrap())
    }

    #[test]
    fn nested_recording_holds_create_and_release_once() {
        let (registry, calls) = fake_registry(false);
        let first = hold_in(registry, HoldReason::Recording); // 0 -> 1
        assert_eq!(
            take(&calls),
            vec![
                format!("create:{APP_NAME} is recording a meeting"),
                "begin".to_string()
            ]
        );
        let second = hold_in(registry, HoldReason::Recording); // 1 -> 2
        drop(first); // 2 -> 1
        assert!(take(&calls).is_empty(), "inner transitions touch nothing");
        drop(second); // 1 -> 0
        assert_eq!(take(&calls), vec!["release", "end"]);
    }

    #[test]
    fn wrap_up_hold_never_takes_the_sleep_assertion() {
        let (registry, calls) = fake_registry(false);
        let wrap_up = hold_in(registry, HoldReason::WrapUp);
        assert_eq!(take(&calls), vec!["begin"]);
        let recording = hold_in(registry, HoldReason::Recording);
        assert_eq!(
            take(&calls),
            vec![format!("create:{APP_NAME} is recording a meeting")]
        );
        drop(recording);
        assert_eq!(
            take(&calls),
            vec!["release"],
            "activity outlives the assertion"
        );
        drop(wrap_up);
        assert_eq!(take(&calls), vec!["end"]);
    }

    #[test]
    fn failing_backend_never_panics_and_still_balances() {
        let (registry, calls) = fake_registry(true);
        let first = hold_in(registry, HoldReason::Recording);
        let second = hold_in(registry, HoldReason::Recording);
        drop(first);
        drop(second);
        // Release calls still happen once each and are no-ops on a failed handle.
        assert_eq!(
            take(&calls),
            vec![
                format!("create:{APP_NAME} is recording a meeting"),
                "begin".to_string(),
                "release".to_string(),
                "end".to_string()
            ]
        );
        assert_eq!(lock(registry).sleep_holds, 0);
        assert_eq!(lock(registry).activity_holds, 0);
    }

    #[test]
    fn recording_slot_holds_once_and_end_is_idempotent() {
        let (registry, calls) = fake_registry(false);
        let slot = Mutex::new(None);
        begin_in(&slot, registry);
        begin_in(&slot, registry); // a second start never stacks a second hold
        assert_eq!(take(&calls).len(), 2, "one create + one begin");
        end_in(&slot);
        assert_eq!(take(&calls), vec!["release", "end"]);
        end_in(&slot); // every stop path may call it; a repeat is a no-op
        assert!(take(&calls).is_empty());
        assert_eq!(lock(registry).sleep_holds, 0);
    }

    #[test]
    fn poisoned_registry_still_releases() {
        let (registry, calls) = fake_registry(false);
        let guard = hold_in(registry, HoldReason::Recording);
        let _ = std::thread::spawn(move || {
            let _l = registry.lock().unwrap();
            panic!("poison the registry");
        })
        .join();
        assert!(registry.is_poisoned());
        take(&calls);
        drop(guard);
        assert_eq!(take(&calls), vec!["release", "end"]);
    }
}
