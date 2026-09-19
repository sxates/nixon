//! Read the Zoom desktop app's self-mute state via macOS Accessibility (specs/0049).
//!
//! Signal (pinned with `scripts/zoom-ax-probe.sh` on a live meeting): Zoom's
//! "Meeting" menu holds a self-mic item titled **"Mute audio"** when you are
//! UNMUTED and **"Unmute audio"** when you are MUTED — distinct from the host
//! items "Mute all" / "Ask all to unmute". We read only that menu item (fast); we
//! never walk window contents (that hangs on Zoom's large meeting window).
//!
//! Reading another app's AX tree needs the Accessibility TCC grant — unlike every
//! other Nixon permission it has no Info.plist key and is prompted at runtime via
//! `AXIsProcessTrustedWithOptions` ([`ensure_ax_trust`]).
//!
//! Reader functions are wired by the mute poll task (specs/0049 Task 3); until then
//! they're unused, hence the module-level dead-code/unused-import allow.
#![allow(dead_code, unused_imports)]

/// Pure: map a Zoom "Meeting"-menu item title to a mute state. `Some(true)` = muted,
/// `Some(false)` = unmuted, `None` = not the self-mic item (host controls like
/// "Mute all" / "Ask all to unmute", or anything else). Trimmed + case-insensitive.
pub(crate) fn mute_state_from_menu_item_title(title: &str) -> Option<bool> {
    match title.trim().to_ascii_lowercase().as_str() {
        "unmute audio" => Some(true), // muted → Zoom offers "Unmute"
        "mute audio" => Some(false),  // unmuted → Zoom offers "Mute"
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos_ax::{ensure_ax_trust, read_zoom_mute_state, zoom_app_pid};

// Non-macOS stubs so the poll monitor needs no cfg. Nixon is macOS-only in practice;
// these keep a cross-platform `cargo check` honest (the feature is simply inert).
#[cfg(not(target_os = "macos"))]
pub(crate) fn ensure_ax_trust(_prompt: bool) -> bool {
    false
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn zoom_app_pid() -> Option<i32> {
    None
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn read_zoom_mute_state(_pid: i32) -> Option<bool> {
    None
}

#[cfg(target_os = "macos")]
mod macos_ax {
    use super::mute_state_from_menu_item_title;
    use accessibility::{AXAttribute, AXUIElement};
    use accessibility_sys::{kAXTrustedCheckOptionPrompt, AXIsProcessTrustedWithOptions};
    use core_foundation::array::CFArray;
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    /// The main Zoom app process on macOS — the one that owns the menu bar, NOT the
    /// `cpthost`/`aomhost` meeting helpers the presence monitor watches.
    const ZOOM_APP_PROCESS: &str = "zoom.us";

    /// Whether this process is trusted for the Accessibility API. When `prompt` is
    /// true and it isn't trusted, macOS shows the "grant Accessibility" system
    /// prompt (there is no Info.plist key — it's a manual System-Settings grant).
    pub(crate) fn ensure_ax_trust(prompt: bool) -> bool {
        // SAFETY: FFI into ApplicationServices; the options dictionary and its
        // CFString key/CFBoolean value outlive the synchronous call.
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let opts = CFDictionary::from_CFType_pairs(&[(
                key.as_CFType(),
                CFBoolean::from(prompt).as_CFType(),
            )]);
            AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
        }
    }

    /// PID of the main Zoom app, if running.
    pub(crate) fn zoom_app_pid() -> Option<i32> {
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
        sys.processes().values().find_map(|p| {
            (p.name().to_string_lossy() == ZOOM_APP_PROCESS).then(|| p.pid().as_u32() as i32)
        })
    }

    fn children(el: &AXUIElement) -> Option<CFArray<AXUIElement>> {
        el.attribute(&AXAttribute::children()).ok()
    }

    fn title_of(el: &AXUIElement) -> Option<String> {
        el.attribute(&AXAttribute::title())
            .ok()
            .map(|s| s.to_string())
    }

    /// Read Zoom's self-mute state from the "Meeting" menu. `Some(true/false)` when
    /// read successfully; `None` when Zoom isn't running / not in a meeting / AX not
    /// permitted / the item wasn't found — callers treat `None` as "unknown; leave
    /// the gate unchanged". Reads only the menu (fast); never walks window contents.
    pub(crate) fn read_zoom_mute_state(pid: i32) -> Option<bool> {
        let app = AXUIElement::application(pid);
        let menu_bar = app
            .attribute(&AXAttribute::new(&CFString::from_static_string(
                "AXMenuBar",
            )))
            .ok()?
            .downcast::<AXUIElement>()?;
        // The "Meeting" menu bar item… (bind each CFArray so the `ItemRef`s that
        // borrow it outlive the statement — chaining `.iter()` on a temporary drops
        // the array while the ref is still held, E0716.)
        let menu_bar_items = children(&menu_bar)?;
        let meeting = menu_bar_items
            .iter()
            .find(|it| title_of(it).as_deref() == Some("Meeting"))?;
        // …its single child is the AXMenu; the menu's children are the items.
        let meeting_children = children(&meeting)?;
        let menu = meeting_children.iter().next()?;
        let menu_items = children(&menu)?;
        menu_items
            .iter()
            .find_map(|item| title_of(&item).and_then(|t| mute_state_from_menu_item_title(&t)))
    }
}

#[cfg(test)]
mod tests {
    use super::mute_state_from_menu_item_title as m;

    #[test]
    fn maps_self_mic_item_titles_to_state() {
        assert_eq!(m("Unmute audio"), Some(true)); // muted → offered "Unmute"
        assert_eq!(m("Mute audio"), Some(false)); // unmuted → offered "Mute"
        assert_eq!(m("  UNMUTE AUDIO "), Some(true)); // trims + case-insensitive
    }

    #[test]
    fn ignores_host_controls_and_other_items() {
        assert_eq!(m("Mute all"), None);
        assert_eq!(m("Ask all to unmute"), None);
        assert_eq!(m("Record to this computer"), None);
        assert_eq!(m(""), None);
    }
}
