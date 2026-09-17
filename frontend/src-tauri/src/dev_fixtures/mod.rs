//! Debug-only fixture seeding and onboarding harness (specs/0059).
//! Compiled out of release builds; every entry point re-checks the `.debug` identifier.
#![cfg(debug_assertions)]

pub mod audio;
pub mod dataset;
pub mod folder;
pub mod guard;
pub mod seed;
pub mod wav;
