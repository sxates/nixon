//! specs/0087 — agent access: the settings file, `nixon://` link formatting and the access
//! log shared by the app and the `nixon --mcp` stdio server. Deliberately free of Tauri
//! types (the MCP server runs outside the app); every function takes the data dir.

pub mod links;
pub mod log;
pub mod queries;
mod queries_search;
pub mod settings;
