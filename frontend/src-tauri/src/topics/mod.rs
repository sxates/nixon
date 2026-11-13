//! Topics (specs/0089): owner-curated threads that collect what meetings said over time.
//!
//! Ledger rule: machines only append (entries, suggestions); the owner alone edits `body_json`
//! and "My notes". `ai_body_json` is written only by insert, by `ledger::rewrite_entry`
//! (owner-triggered, while unedited) and by accept-merge in `ledger::resolve_suggestion`
//! (appends one bullet). `notes_*` has one writer (`repo::set_notes`). Deleting a source
//! meeting keeps its entries, marked `source_deleted_at`.

pub mod evidence;
pub mod ledger;
pub mod limits;
pub mod links;
pub mod model;
pub mod prompts;
pub mod repo;

pub use repo::{emit_updated, TOPICS_UPDATED_EVENT};
