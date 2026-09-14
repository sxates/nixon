// Settings domain: model/transcript provider config + API-key status commands
// (specs/0042 WS3 — dispersed from the old api/api.rs grab-bag).

pub mod commands;
pub mod custom_openai;

pub use commands::*;
pub use custom_openai::*;
