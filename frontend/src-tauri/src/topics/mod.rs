//! Topics (specs/0089): owner-curated threads that collect what meetings said over time.
//!
//! Ledger rule: machines only append (entries, suggestions); the owner alone edits `body_json`
//! and "My notes". `ai_body_json` is never written after insert, and `notes_*` has one writer
//! (`repo::set_notes`). Deleting a source meeting keeps its entries, marked `source_deleted_at`.

pub mod limits;
pub mod links;
pub mod model;
pub mod repo;

pub use repo::{emit_updated, TOPICS_UPDATED_EVENT};
