#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

// NOTE: logging is initialized by `tauri-plugin-log` inside `app_lib::run()`
// (registered in lib.rs), which installs the global `log` logger and writes to
// BOTH a file (OS app-log dir) and stdout. We deliberately do NOT call
// `env_logger::init()` here: two `log` loggers conflict and panic, and the file
// target is what makes the bundled .app debuggable. RUST_LOG can still tune
// per-module levels at runtime if set in the environment.
fn main() {
    // specs/0087: `nixon --mcp` is the read-only MCP server Claude Code launches. It returns
    // before `app_lib::run()`, so no window, Dock icon, single-instance hand-off or logger
    // (tauri-plugin-log writes to stdout, which carries only JSON-RPC in this mode).
    let args: Vec<String> = std::env::args().collect();
    match app_lib::agent_access::entry::parse_mcp_args(&args) {
        Some(Ok(mcp)) => std::process::exit(app_lib::agent_access::entry::run_stdio(mcp)),
        Some(Err(e)) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
        None => {}
    }
    // `log::info!` before the plugin installs the logger is a no-op (no global
    // logger yet); the plugin logs "Application setup complete" once it's up.
    app_lib::run();
}
