// Calendar integration (specs/0008 P2, specs/0032).
//
// Two providers, ONE active source at a time (specs/0032, single-active-source
// model — see [`google_is_active_source`]):
//   - EventKit — the user's local macOS Calendar (which already aggregates
//     their Exchange / iCloud / CalDAV accounts). 100% on-device: EventKit
//     makes no network calls, matching Nixon's privacy-first design. Read-only.
//   - Google Calendar (`google/`) — an opt-in, read-only API sync into a local
//     SQLite cache. While an account is connected it REPLACES EventKit as the
//     calendar source (no merging across sources).
//
// Layout mirrors `src/zoom/`:
//   - `eventkit.rs` — the EventKit (`objc2-event-kit`) FFI reader + auth bridge.
//   - `zoom_link.rs` — regex extraction of a Zoom join URL from event text.
//   - `commands.rs`  — Tauri commands registered in `lib.rs`.
//   - `google/`     — the Google Calendar OAuth + sync provider (specs/0032).
//
// PERMISSION / Info.plist (REQUIRED): requesting calendar access *crashes* the
// process unless the bundled app's Info.plist contains
// `NSCalendarsFullAccessUsageDescription` (macOS 14+) and the legacy
// `NSCalendarsUsageDescription`. Those are added to `src-tauri/Info.plist`,
// which Tauri merges into the built `.app`. A bare `cargo run` dev binary has
// no Info.plist, so the access prompt (and therefore reading events) only works
// from a bundled `.app` build — `api_get_calendar_access_status` is harmless to
// call from the dev binary (it just reports notDetermined/denied).

pub mod commands;
pub mod day_agenda;
pub mod eventkit;
pub mod google;
pub mod zoom_link;

/// Which calendar source is active (specs/0032, single-active-source model):
/// while a Google account is connected, the local Google cache is the ONLY
/// calendar source — no EventKit reads, no macOS Calendar permission
/// dependency. When not connected, EventKit is the only source — exactly the
/// pre-Google behavior. Every calendar consumer (agenda, upcoming, title+time
/// attendee fallbacks) routes through this one decision.
///
/// Best-effort by design: an unconfigured build, an uninitialized DB (first
/// launch), or a failed account-state read all select EventKit — never an
/// error, so calendar reads stay non-fatal everywhere.
pub async fn google_is_active_source<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    if !google::is_configured() {
        return false;
    }
    let Some(pool) = google::sync::db_pool(app) else {
        return false;
    };
    matches!(
        crate::database::repositories::google_calendar::GoogleCalendarRepository::get_account(
            &pool
        )
        .await,
        Ok(Some(_))
    )
}
