//! Google Calendar provider (specs/0032, ADR-0010) — Nixon's first sanctioned
//! non-LLM egress: an **opt-in, read-only** connection to the Google Calendar
//! API. Only event *metadata* is downloaded; nothing the app produces
//! (audio, transcripts, notes, summaries) is ever uploaded.
//!
//! Layout:
//!   - `oauth.rs` — authorization-code + PKCE loopback flow, token
//!     refresh/revoke, Keychain persistence of the refresh token.
//!   - `sync.rs` — poll-based incremental event sync into the
//!     `google_calendar_*` SQLite cache, plus the cache readers that serve as
//!     THE calendar feed while connected (single-active-source model — see
//!     [`crate::calendar::google_is_active_source`]) and the attendee-routing
//!     lookups.
//!   - `photos.rs` — the best-effort attendee photo / directory-name pass that
//!     runs at the end of a sync.
//!   - `commands.rs` — the `api_google_calendar_*` Tauri IPC layer and the
//!     `google-calendar-auth-required` event contract.
//!
//! **Client credentials** are baked in at compile time from the build
//! environment (`NIXON_GOOGLE_CLIENT_ID` / `NIXON_GOOGLE_CLIENT_SECRET`).
//! Google designates a desktop-app client secret as *non-confidential*
//! (ADR-0010) — PKCE is the actual protection. When the id is unset the
//! feature is inert: [`is_configured`] returns `false` and the Settings UI
//! renders a "not configured in this build" state instead of a Connect button.

pub mod capabilities;
pub mod cloud_identity;
pub mod commands;
/// `events.list` wire types + event→row mapping, split out of `sync.rs` (specs/0054 W5).
pub(crate) mod events_map;
pub mod oauth;
pub mod people_api;
/// Attendee photo enrichment pass, split out of `sync.rs` (specs/0056 W6).
pub(crate) mod photos;
pub mod sync;

/// Compile-time OAuth client id (Google Cloud "Desktop app" client).
const CLIENT_ID: Option<&str> = option_env!("NIXON_GOOGLE_CLIENT_ID");
/// Compile-time OAuth client secret (non-confidential for desktop clients).
const CLIENT_SECRET: Option<&str> = option_env!("NIXON_GOOGLE_CLIENT_SECRET");

/// The baked-in client id, or `None` when this build wasn't configured.
/// (Empty/whitespace values count as unset so a stray `NIXON_GOOGLE_CLIENT_ID=`
/// in a build script doesn't produce a broken Connect button.)
pub(crate) fn client_id() -> Option<&'static str> {
    CLIENT_ID.map(str::trim).filter(|s| !s.is_empty())
}

/// The baked-in client secret, if any (Google issues one even for desktop
/// clients; it rides along in the token exchange when present).
pub(crate) fn client_secret() -> Option<&'static str> {
    CLIENT_SECRET.map(str::trim).filter(|s| !s.is_empty())
}

/// True when OAuth client credentials were baked into this build — the gate
/// for showing the Connect flow at all (spec 0032 acceptance criterion 9).
pub fn is_configured() -> bool {
    client_id().is_some()
}

#[cfg(test)]
mod tests {
    // `option_env!` is fixed at compile time, so `is_configured` itself can't be
    // exercised both ways in one binary; the filtering helpers carry the logic.
    #[test]
    fn is_configured_matches_client_id_presence() {
        assert_eq!(super::is_configured(), super::client_id().is_some());
    }
}
