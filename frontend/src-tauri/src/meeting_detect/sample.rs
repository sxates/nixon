// The monitor's raw inputs (specs/0074 W6): Zoom's meeting helpers, the Core Audio client
// process list, and (only with Accessibility already granted) a browser's front window title.
//
// Blocking calls (sysinfo, Core Audio property reads, AX) — the monitor runs them on the
// blocking pool, never on the async executor.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::classify::AudioProc;

/// Zoom meeting-helper process names that exist ONLY during an active meeting
/// (case-insensitive match). `caphost` was removed — it's Zoom's screen-share/capture
/// helper that keeps running while Zoom is open with NO meeting (verified: idle → only
/// caphost present), which caused false "meeting detected" prompts. `cpthost` (caption
/// host) and `aomhost` (audio) spawn for an actual meeting and exit on leave. Isolated
/// here so a Zoom rename is a one-line change.
pub(crate) const ZOOM_MEETING_PROCESSES: [&str; 2] = ["cpthost", "aomhost"];

/// Safari's media process is launched by launchd, not by Safari, so its parent pid does not
/// lead to the app; the app is found by name instead.
const SAFARI_PROCESS: &str = "Safari";

/// Refresh process names only (the cheapest refresh — no cpu/mem/disk/env).
pub(crate) fn refresh(sys: &mut System) {
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
}

/// True if a Zoom meeting-helper process is running. Call after [`refresh`].
pub(crate) fn zoom_meeting_process_present(sys: &System) -> bool {
    sys.processes().values().any(|proc_| {
        let name = proc_.name().to_string_lossy();
        ZOOM_MEETING_PROCESSES
            .iter()
            .any(|target| name.eq_ignore_ascii_case(target))
    })
}

/// Every Core Audio client process with its bundle id and whether it is capturing input.
/// Reading these properties needs no permission (macOS 14+). A process whose properties
/// cannot be read (it exited mid-read) is skipped; a failed list is an empty one.
#[cfg(target_os = "macos")]
pub fn audio_processes() -> Vec<AudioProc> {
    use cidre::core_audio as ca;
    let list = match ca::Process::list() {
        Ok(list) => list,
        Err(e) => {
            log::debug!("meeting detect: Core Audio process list unavailable: {e:?}");
            return Vec::new();
        }
    };
    list.iter()
        .filter_map(|p| {
            Some(AudioProc {
                pid: p.pid().ok()?,
                bundle_id: p.bundle_id().map(|b| b.to_string()).unwrap_or_default(),
                running_input: p.is_running_input().unwrap_or(false),
            })
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub fn audio_processes() -> Vec<AudioProc> {
    Vec::new()
}

/// The browser app that owns a microphone-holding helper: the helper's parent for Chromium
/// browsers and Firefox, the Safari process for WebKit's shared media process. Call after
/// [`refresh`].
pub(crate) fn browser_app_pid(sys: &System, helper_pid: i32) -> Option<i32> {
    let helper = sys.process(Pid::from_u32(helper_pid as u32))?;
    let by_parent = helper
        .parent()
        .filter(|parent| parent.as_u32() > 1)
        .map(|parent| parent.as_u32() as i32);
    by_parent.or_else(|| {
        sys.processes().values().find_map(|p| {
            (p.name().to_string_lossy() == SAFARI_PROCESS).then(|| p.pid().as_u32() as i32)
        })
    })
}

/// The front window title of `app_pid`, ONLY when Accessibility is already granted.
/// `ensure_ax_trust(false)` is `AXIsProcessTrustedWithOptions` without the prompt option:
/// it answers the question and never asks the user anything.
#[cfg(target_os = "macos")]
pub(crate) fn front_window_title(app_pid: i32) -> Option<String> {
    use accessibility::{AXAttribute, AXUIElement};
    use core_foundation::string::CFString;

    if !crate::zoom::mute::ensure_ax_trust(false) {
        return None;
    }
    let app = AXUIElement::application(app_pid);
    let window = app
        .attribute(&AXAttribute::new(&CFString::from_static_string(
            "AXFocusedWindow",
        )))
        .ok()?
        .downcast::<AXUIElement>()?;
    window
        .attribute(&AXAttribute::title())
        .ok()
        .map(|t| t.to_string())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn front_window_title(_app_pid: i32) -> Option<String> {
    None
}
