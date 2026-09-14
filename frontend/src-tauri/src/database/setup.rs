use log::info;
use tauri::{AppHandle, Emitter, Manager};

use super::manager::DatabaseManager;
use crate::state::AppState;

/// Assert that the SQLite library sqlx linked against was compiled with FTS5
/// (specs/0033). The full-text search migration (`20260706000000_add_fts5_search.sql`)
/// would fail loudly on its own if FTS5 were missing — this probe just makes the
/// error message say *why*, before `sqlx::migrate!` runs.
///
/// FTS5 availability is a compile-time property of the bundled SQLite
/// (`libsqlite3-sys` with the `bundled` feature builds with `-DSQLITE_ENABLE_FTS5`),
/// so probing a throwaway in-memory connection proves it for every connection.
/// A `PRAGMA compile_options` check is flakier across bundled builds; actually
/// creating a virtual table is the authoritative test.
pub async fn assert_fts5_available() -> Result<(), String> {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::{ConnectOptions, Connection};

    let mut conn = SqliteConnectOptions::new()
        .in_memory(true)
        .connect()
        .await
        .map_err(|e| format!("FTS5 probe: failed to open an in-memory SQLite connection: {e}"))?;

    let probe = sqlx::query("CREATE VIRTUAL TABLE temp.__fts5_probe USING fts5(x)")
        .execute(&mut conn)
        .await;
    // Best-effort cleanup; the connection is throwaway either way.
    let _ = sqlx::query("DROP TABLE IF EXISTS temp.__fts5_probe")
        .execute(&mut conn)
        .await;
    let _ = conn.close().await;

    probe.map(|_| ()).map_err(|e| {
        format!(
            "This build's SQLite library was compiled WITHOUT FTS5, so full-text \
             search (and its database migration) cannot work. Nixon expects the \
             bundled SQLite from libsqlite3-sys (`sqlx` with the `sqlite` feature), \
             which enables SQLITE_ENABLE_FTS5 — a dependency or feature-flag change \
             has likely swapped in a different SQLite. Probe error: {e}"
        )
    })
}

/// Initialize database on app startup
/// Handles first launch detection and conditional initialization
pub async fn initialize_database_on_startup(app: &AppHandle) -> Result<(), String> {
    // Fail loudly (and explain why) if FTS5 is missing, BEFORE any migration runs
    // for either the first-launch or normal path (specs/0033).
    assert_fts5_available().await?;

    // Check if this is the first launch (no database exists yet)
    let is_first_launch = DatabaseManager::is_first_launch(app)
        .await
        .map_err(|e| format!("Failed to check first launch status: {}", e))?;

    if is_first_launch {
        info!("First launch detected - will notify window when ready");

        // Delay event emission to ensure window is ready and React listeners are registered
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            app_handle
                .emit("first-launch-detected", ())
                .expect("Failed to emit first-launch-detected event");
            info!("Emitted first-launch-detected after delay");
        });
    } else {
        // Normal flow - initialize database immediately
        let db_manager = DatabaseManager::new_from_app_handle(app)
            .await
            .map_err(|e| format!("Failed to initialize database manager: {}", e))?;

        // One-time (idempotent) move of any cleartext API keys into the Keychain
        // (spec 0030 WS1 / ADR-0009). Best-effort: failures leave keys readable
        // from the DB and retry next launch.
        crate::database::repositories::setting::SettingsRepository::
            migrate_keys_to_keychain_best_effort(db_manager.pool())
        .await;

        app.manage(AppState { db_manager });
        info!("Database initialized successfully");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    /// specs/0033: the bundled SQLite (libsqlite3-sys `bundled` feature via sqlx's
    /// `sqlite` feature) compiles with -DSQLITE_ENABLE_FTS5. This locks that
    /// dependency in CI so a future sqlx/feature change that drops FTS5 fails
    /// loudly here instead of silently breaking search.
    #[tokio::test]
    async fn bundled_sqlite_has_fts5() {
        super::assert_fts5_available()
            .await
            .expect("bundled SQLite must be compiled with FTS5 (see specs/0033)");
    }
}
