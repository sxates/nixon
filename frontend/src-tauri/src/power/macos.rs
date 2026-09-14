//! IOKit power-source FFI (macOS). Kept thin and unsafe-contained; the
//! decision logic lives in `mod.rs` / `audio::processing_mode` where it is
//! testable without hardware.

use super::PowerSource;
use std::ffi::{c_char, c_void, CStr};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type CFRunLoopRef = *const c_void;
type CFIndex = isize;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSGetProvidingPowerSourceType(snapshot: CFTypeRef) -> CFStringRef;
    fn IOPSNotificationCreateRunLoopSource(
        callback: extern "C" fn(context: *mut c_void),
        context: *mut c_void,
    ) -> CFRunLoopSourceRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRelease(cf: CFTypeRef);
    fn CFStringGetCString(s: CFStringRef, buffer: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    static kCFRunLoopDefaultMode: CFStringRef;
}

/// Query the providing power source ("AC Power" / "Battery Power" / "UPS Power").
pub(super) fn query_power_source() -> PowerSource {
    // SAFETY: `IOPSCopyPowerSourcesInfo` returns either null or an owned
    // CFTypeRef we release below; `IOPSGetProvidingPowerSourceType` returns a
    // borrowed (not owned) CFStringRef per the Get-rule, valid for the
    // lifetime of `snapshot`. `CFStringGetCString` is given a fixed-size
    // stack buffer and its exact length, and we only read from it after
    // checking the success return value.
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return PowerSource::Unknown;
        }
        // Get-rule: the returned CFString is NOT owned by us; only the
        // snapshot needs releasing.
        let source_type = IOPSGetProvidingPowerSourceType(snapshot);
        let result = if source_type.is_null() {
            PowerSource::Unknown
        } else {
            let mut buf = [0 as c_char; 64];
            if CFStringGetCString(source_type, buf.as_mut_ptr(), 64, K_CF_STRING_ENCODING_UTF8) != 0
            {
                match CStr::from_ptr(buf.as_ptr()).to_string_lossy().as_ref() {
                    "Battery Power" => PowerSource::Battery,
                    "AC Power" | "UPS Power" => PowerSource::Ac,
                    _ => PowerSource::Unknown,
                }
            } else {
                PowerSource::Unknown
            }
        };
        CFRelease(snapshot);
        result
    }
}

/// Park a dedicated thread in a CFRunLoop that fires `on_change` on every
/// power-source notification. The closure is intentionally leaked — the
/// monitor lives for the whole process.
pub(super) fn spawn_notification_thread(on_change: impl Fn() + Send + 'static) {
    extern "C" fn trampoline(context: *mut c_void) {
        // SAFETY: `context` was produced by `Box::into_raw` below from a
        // `Box<Box<dyn Fn() + Send>>` and is never freed or reused for the
        // life of the process, so the reference stays valid for every call.
        let cb = unsafe { &*(context as *const Box<dyn Fn() + Send>) };
        cb();
    }
    let boxed: Box<Box<dyn Fn() + Send>> = Box::new(Box::new(on_change));
    // Pass the pointer as usize so the spawned closure is Send.
    let context = Box::into_raw(boxed) as usize;
    if let Err(e) = std::thread::Builder::new()
        .name("power-monitor".into())
        .spawn(move || {
            // SAFETY: called once on a dedicated thread; `context` is a live
            // pointer for the whole process (see above), and the CFRunLoop
            // APIs are safe to call from any thread as long as they're not
            // re-entered concurrently on the same run loop, which holds here
            // since this thread owns its own run loop.
            unsafe {
                let source =
                    IOPSNotificationCreateRunLoopSource(trampoline, context as *mut c_void);
                if source.is_null() {
                    log::warn!("power monitor: IOPSNotificationCreateRunLoopSource failed; low-power auto-detection disabled");
                    return;
                }
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
                CFRunLoopRun();
            }
        })
    {
        log::warn!("power monitor thread failed to spawn: {e}");
    }
}
