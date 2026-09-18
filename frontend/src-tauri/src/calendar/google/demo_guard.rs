//! Keeps the developer's real calendar out of a demo profile (specs/0060 follow-up).
//!
//! `./dev-nixon.sh --demo` seeds the fictional dataset into the `.debug` profile, and
//! that profile is what `pnpm shots:real` captures the README screenshots from. The
//! fixture seeder wipes and re-seeds meetings and people, but the Google Calendar
//! cache used to survive it and render straight through: the first real capture run
//! published 19 real contacts - names, work addresses and face photos - plus real
//! meeting titles on the Today timeline.
//!
//! Two halves fix that, and both are needed. The seeder clears what previous runs
//! cached (`dev_fixtures::seed`'s real-data cache tables); this module stops the sync
//! that would immediately refill it. The connected account itself is left alone, so
//! no re-authentication is needed - only the ingest is suppressed, and only while the
//! demo dataset is the active dataset.

/// True when calendar sync must not run because the demo dataset is active.
///
/// The whole `dev_fixtures` module is `#![cfg(debug_assertions)]`, so its guard
/// cannot be named unconditionally. In a release build the demo dataset does not
/// exist and this is a compile-time `false`.
pub(super) fn sync_suppressed() -> bool {
    #[cfg(debug_assertions)]
    {
        let suppressed = crate::dev_fixtures::guard::demo_dataset_active();
        if suppressed {
            log::info!("google calendar: sync suppressed (demo dataset active)");
        }
        suppressed
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn is_false_outside_a_demo_debug_bundle() {
        // `cargo test` builds with debug_assertions on, so this runs the real arm:
        // the bundle identifier OnceLock is never populated outside the Tauri setup
        // hook, so `demo_dataset_active()` fails closed and sync stays enabled.
        assert!(!super::sync_suppressed());
    }
}
