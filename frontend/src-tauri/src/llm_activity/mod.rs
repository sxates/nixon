//! Background LLM activity tracking (specs/0052).
//!
//! Background work (prep-brief generation, action-item extraction) previously ran with no UI
//! signal at all — a prep brief failed every 30 minutes for months while the fans spun, and
//! nothing in the app said so. This module records what background LLM work is running and
//! how it ended, so the sidebar can show it.

pub mod commands;
pub mod registry;

pub use registry::{
    LlmActivityState, LlmActivityView, LlmTaskRegistry, Origin, TaskHandle, TaskKind,
};
