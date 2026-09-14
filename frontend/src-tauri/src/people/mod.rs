//! The durable, app-wide People entity (specs/0016 Phase 1b).
//!
//! A `people` row anchors one human across meetings; `speakers.person_id` links a
//! per-meeting speaker occurrence to it. Identity is decoupled from voiceprints
//! (ADR-0007 §2) — assigning a person to a speaker works regardless of the per-person
//! `voiceprint_opt_out` flag. The DB I/O lives in
//! [`crate::database::repositories::people::PeopleRepository`]; this module is the Tauri
//! command surface registered in `lib.rs`.

pub mod commands;
pub mod enroll;
pub mod merge;
