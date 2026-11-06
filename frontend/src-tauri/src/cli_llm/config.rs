//! The user's Locate... override for each subscription CLI (specs/0085), stored as JSON in
//! `settings.cliProviderConfig`: `{"claude-code":{"binaryPath":null|"/..."},"codex-cli":{...}}`.
//! Same pattern as `customOpenAIConfig`, kept here rather than in the (full)
//! `SettingsRepository`. Only the override lives in the DB; detection results are in memory.
//!
//! `llm_client` has no pool, so [`user_override`] answers from an in-memory mirror that is
//! filled from the DB at startup ([`load`]) and kept current by [`set_binary_path`].

use super::CliKind;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(rename = "binaryPath", default)]
    binary_path: Option<String>,
}

type Config = BTreeMap<String, Entry>;

static MIRROR: Lazy<Mutex<HashMap<CliKind, PathBuf>>> = Lazy::new(|| Mutex::new(HashMap::new()));

const KINDS: [CliKind; 2] = [CliKind::ClaudeCode, CliKind::CodexCli];

async fn read(pool: &SqlitePool) -> Result<Config, sqlx::Error> {
    let json: Option<Option<String>> =
        sqlx::query_scalar("SELECT cliProviderConfig FROM settings WHERE id = '1'")
            .fetch_optional(pool)
            .await?;
    // Unreadable JSON is treated as "no overrides" rather than breaking every LLM call.
    Ok(json
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}

/// The saved override for `kind`, if any.
pub async fn binary_path(pool: &SqlitePool, kind: CliKind) -> Result<Option<String>, sqlx::Error> {
    Ok(read(pool)
        .await?
        .remove(kind.id())
        .and_then(|e| e.binary_path))
}

/// What Locate… says on a database that has no settings row yet (onboarding not done).
pub const NO_SETTINGS_ROW: &str = "Finish setting up Nixon first, then choose Claude Code.";

/// [`set_binary_path`] found no settings row to store the override in. Its message is
/// [`NO_SETTINGS_ROW`], shown to the user as is.
#[derive(Debug)]
pub struct NoSettingsRow;

impl std::fmt::Display for NoSettingsRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(NO_SETTINGS_ROW)
    }
}

impl std::error::Error for NoSettingsRow {}

/// Saves (or, with `None`, clears) the override for `kind`, leaving the other CLI's entry
/// and the saved provider/model alone, and updates the in-memory mirror.
///
/// It never creates the settings row: inventing one would save a provider the user never
/// chose. With no row, saving a path fails with [`NoSettingsRow`]; clearing is a no-op.
pub async fn set_binary_path(
    pool: &SqlitePool,
    kind: CliKind,
    path: Option<&str>,
) -> anyhow::Result<()> {
    let mut config = read(pool).await?;
    config.entry(kind.id().to_string()).or_default().binary_path = path.map(str::to_string);
    let json = serde_json::to_string(&config)
        .map_err(|e| anyhow::anyhow!("Failed to serialize cliProviderConfig: {e}"))?;
    let updated = sqlx::query("UPDATE settings SET cliProviderConfig = ?1 WHERE id = '1'")
        .bind(&json)
        .execute(pool)
        .await?
        .rows_affected();
    if updated == 0 && path.is_some() {
        return Err(NoSettingsRow.into());
    }
    mirror(kind, path);
    Ok(())
}

/// The provider currently chosen in Settings, if a settings row exists.
pub async fn saved_provider(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT provider FROM settings WHERE id = '1'")
        .fetch_optional(pool)
        .await
}

/// Fills the mirror from the DB. Call once after the database opens.
pub async fn load(pool: &SqlitePool) {
    match read(pool).await {
        Ok(config) => {
            for kind in KINDS {
                let path = config.get(kind.id()).and_then(|e| e.binary_path.as_deref());
                mirror(kind, path);
            }
        }
        Err(e) => log::warn!("Could not read CLI provider settings: {e}"),
    }
}

fn mirror(kind: CliKind, path: Option<&str>) {
    if let Ok(mut m) = MIRROR.lock() {
        match path {
            Some(p) => m.insert(kind, PathBuf::from(p)),
            None => m.remove(&kind),
        };
    }
}

/// The Locate... path for `kind`, as `generate` needs it.
pub fn user_override(kind: CliKind) -> Option<PathBuf> {
    MIRROR.lock().ok()?.get(&kind).cloned()
}
