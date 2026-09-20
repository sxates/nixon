// Notifications.
//
// Everything here is the specs/0068 path: a native `UNUserNotificationCenter` layer and the
// commands that front it. The 0008-era module that used to live beside it — a manager, a
// settings store, a DND model and fourteen registered commands — was deleted with that spec:
// it delivered through `tauri-plugin-notification`, which on desktop has no action buttons
// and reports permission as granted without asking macOS, and by the end the frontend called
// two of its commands, both of which wrote settings that nothing read.

/// Native macOS notifications — the path that actually delivers.
pub mod macos;
pub mod os_commands;
