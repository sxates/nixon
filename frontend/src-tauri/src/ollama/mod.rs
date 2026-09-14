pub mod commands;
pub mod metadata;
#[allow(clippy::module_inception)] // submodule intentionally shares the parent module name
pub mod ollama;
pub mod served_context;

pub use ollama::*;
// Don't re-export commands to avoid conflicts - lib.rs will import directly
