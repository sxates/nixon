pub mod commands;
#[allow(clippy::module_inception)] // submodule intentionally shares the parent module name
pub mod openrouter;

pub use openrouter::*;
// Don't re-export commands to avoid conflicts - lib.rs will import directly
