//! specs/0083 W1: IOKit power-assertion + `NSProcessInfo` activity FFI (macOS). Kept thin; the
//! refcounting lives in `keep_awake.rs` where it is testable without hardware.

use std::ffi::{c_char, c_void, CString};

use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

use super::keep_awake::KeepAwakeBackend;

type CFStringRef = *const c_void;
type CFAllocatorRef = *const c_void;
type IOPMAssertionID = u32;
type IOReturn = i32;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
const K_IO_RETURN_SUCCESS: IOReturn = 0;
/// `kIOPMAssertionTypePreventUserIdleSystemSleep`: the system stays awake; the display may
/// still sleep.
const ASSERTION_TYPE: &str = "PreventUserIdleSystemSleep";

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        level: u32,
        name: CFStringRef,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

/// An owned CFString, released on drop.
struct OwnedCFString(CFStringRef);

impl OwnedCFString {
    fn new(s: &str) -> Result<Self, String> {
        let c = CString::new(s).map_err(|e| e.to_string())?;
        // SAFETY: `c` is a valid NUL-terminated UTF-8 buffer for the duration of the call;
        // the returned CFString (Create-rule) is owned by us and released in `Drop`.
        let cf = unsafe {
            CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        if cf.is_null() {
            Err(format!("CFStringCreateWithCString failed for {s:?}"))
        } else {
            Ok(Self(cf))
        }
    }
}

impl Drop for OwnedCFString {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a non-null CFString we own (checked in `new`).
        unsafe { CFRelease(self.0) }
    }
}

/// The token `beginActivityWithOptions:reason:` returns. `NSProcessInfo`'s activity API is
/// thread-safe, so ending it from whichever thread drops the last hold is fine.
struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

// SAFETY: the token is an opaque object only ever passed back to `endActivity:`, which
// Foundation documents as callable from any thread.
unsafe impl Send for Activity {}

#[derive(Default)]
pub(super) struct MacBackend {
    assertion: Option<IOPMAssertionID>,
    activity: Option<Activity>,
}

impl KeepAwakeBackend for MacBackend {
    fn create_sleep_assertion(&mut self, name: &str) -> Result<(), String> {
        if self.assertion.is_some() {
            return Ok(());
        }
        let kind = OwnedCFString::new(ASSERTION_TYPE)?;
        let name = OwnedCFString::new(name)?;
        let mut id: IOPMAssertionID = 0;
        // SAFETY: both CFStrings are live for the call (IOKit copies them); `id` is a valid
        // out-pointer to a stack u32.
        let rc = unsafe {
            IOPMAssertionCreateWithName(kind.0, K_IOPM_ASSERTION_LEVEL_ON, name.0, &mut id)
        };
        if rc == K_IO_RETURN_SUCCESS {
            self.assertion = Some(id);
            Ok(())
        } else {
            Err(format!("IOPMAssertionCreateWithName returned {rc:#x}"))
        }
    }

    fn release_sleep_assertion(&mut self) {
        if let Some(id) = self.assertion.take() {
            // SAFETY: `id` came from a successful `IOPMAssertionCreateWithName` and is
            // released exactly once (taken out of the Option).
            let rc = unsafe { IOPMAssertionRelease(id) };
            if rc != K_IO_RETURN_SUCCESS {
                log::warn!("keep-awake: IOPMAssertionRelease returned {rc:#x}");
            }
        }
    }

    fn begin_activity(&mut self, reason: &str) -> Result<(), String> {
        if self.activity.is_some() {
            return Ok(());
        }
        let token = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &NSString::from_str(reason),
        );
        self.activity = Some(Activity(token));
        Ok(())
    }

    fn end_activity(&mut self) {
        if let Some(Activity(token)) = self.activity.take() {
            // SAFETY: `token` is the object `beginActivityWithOptions:reason:` returned, ended
            // exactly once.
            unsafe { NSProcessInfo::processInfo().endActivity(&token) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assertions() -> String {
        let out = std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .expect("pmset runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Real-OS check (specs/0083 acceptance A): run with
    /// `cargo test --features metal --lib keep_awake_macos -- --ignored --nocapture`.
    #[test]
    #[ignore = "touches the real power-management state; run manually"]
    fn real_assertion_shows_in_pmset_and_goes_on_release() {
        let name = format!(
            "{} keep-awake self-test {}",
            crate::app_paths::APP_NAME,
            std::process::id()
        );
        let mut backend = MacBackend::default();
        backend
            .create_sleep_assertion(&name)
            .expect("assertion created");
        backend.begin_activity(&name).expect("activity begun");
        let held = assertions();
        let line = held.lines().find(|l| l.contains(&name)).map(str::to_owned);
        println!("while held: {line:?}");
        assert!(
            line.as_deref()
                .is_some_and(|l| l.contains("PreventUserIdleSystemSleep")),
            "pmset should list the named assertion:\n{held}"
        );
        backend.release_sleep_assertion();
        backend.end_activity();
        let after = assertions();
        println!(
            "after release, named lines: {:?}",
            after
                .lines()
                .filter(|l| l.contains(&name))
                .collect::<Vec<_>>()
        );
        assert!(!after.contains(&name), "assertion should be gone:\n{after}");
    }
}
