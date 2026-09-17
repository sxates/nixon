//! Debug-only fixture seeding and onboarding harness (specs/0059).
//! Compiled out of release builds; every entry point re-checks the `.debug` identifier.
#![cfg(debug_assertions)]

pub mod dataset;
