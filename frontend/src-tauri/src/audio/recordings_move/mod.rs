//! specs/0073 W2 — the recordings mover.
//!
//! One recordings folder, always: changing the folder moves every meeting's recordings
//! there, and the startup gather collects any meeting still outside it. Each folder moves
//! under its meetings' folder leases (`audio::folder_lease`), with a journal and an ordering
//! that make every crash point recoverable. See `exec.rs` for the recovery table and
//! `roots.rs` for which folders a build profile may move from or remove.

pub mod commands;
pub mod disk;
pub mod exec;
pub mod journal;
pub mod plan;
pub mod roots;
pub mod runner;

#[cfg(test)]
mod plan_tests;
