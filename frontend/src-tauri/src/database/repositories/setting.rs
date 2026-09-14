use crate::database::models::{Setting, TranscriptSetting};
use crate::summary::CustomOpenAIConfig;
use sqlx::SqlitePool;

#[derive(serde::Deserialize, Debug)]
pub struct SaveModelConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "ollamaEndpoint")]
    pub ollama_endpoint: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
pub struct SaveTranscriptConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
}

pub struct SettingsRepository;

// API keys (spec 0030 WS1 / ADR-0009): the macOS Keychain is the primary store; the
// legacy plaintext columns in `settings` / `transcript_settings` (and the api_key
// nested in the custom-openai JSON blob) now hold either the Keychain sentinel
// (`__keychain_v1__`) or a not-yet-migrated cleartext key. All reads/writes route
// through `crate::secrets`:
//   - reads prefer the Keychain and fall back to a non-sentinel DB value; a sentinel
//     with an unreadable Keychain item = provider NOT CONFIGURED (Ok(None), no error);
//   - writes go Keychain-first and only fall back to cleartext-in-DB when the
//     Keychain write fails (re-migrated on a later launch);
//   - `migrate_keys_to_keychain` runs at startup and idempotently moves any
//     remaining cleartext values over.
// The 0028 interim mitigation (0600 perms via `DatabaseManager::harden_db_permissions`)
// stays as defense-in-depth for the fallback values.

// Transcript providers: localWhisper, deepgram, elevenLabs, groq, openai
// Summary providers: openai, claude, ollama, groq, added openrouter
// NOTE: Handle data exclusion in the higher layer as this is database abstraction layer(using SELECT *)

impl SettingsRepository {
    pub async fn get_model_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<Setting>, sqlx::Error> {
        let setting = sqlx::query_as::<_, Setting>("SELECT * FROM settings LIMIT 1")
            .fetch_optional(pool)
            .await?;
        Ok(setting)
    }

    pub async fn save_model_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
        whisper_model: &str,
        ollama_endpoint: Option<&str>,
    ) -> std::result::Result<(), sqlx::Error> {
        // Using id '1' for backward compatibility
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, ollamaEndpoint)
            VALUES ('1', $1, $2, $3, $4)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model,
                whisperModel = excluded.whisperModel,
                ollamaEndpoint = excluded.ollamaEndpoint
            "#,
        )
        .bind(provider)
        .bind(model)
        .bind(whisper_model)
        .bind(ollama_endpoint)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_api_key(
        pool: &SqlitePool,
        provider: &str,
        api_key: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Custom OpenAI uses JSON config (customOpenAIConfig) instead of a separate API key column
        if provider == "custom-openai" {
            return Err(sqlx::Error::Protocol(
                "custom-openai provider should use save_custom_openai_config() instead of save_api_key()".into(),
            ));
        }

        // A masked hint or sentinel echoed back by the frontend is not a key —
        // ignore it so the stored key is never clobbered (ADR-0009).
        if crate::secrets::is_placeholder(api_key) {
            log::debug!("save_api_key: ignoring placeholder value for provider '{provider}'");
            return Ok(());
        }

        let api_key_column = match provider {
            "openai" => "openaiApiKey",
            "claude" => "anthropicApiKey",
            "ollama" => "ollamaApiKey",
            "groq" => "groqApiKey",
            "openrouter" => "openRouterApiKey",
            "builtin-ai" => return Ok(()), // No API key needed
            _ => {
                return Err(sqlx::Error::Protocol(format!(
                    "Invalid provider: {}",
                    provider
                )))
            }
        };

        // Keychain-first (ADR-0009): on success the DB column only holds the
        // sentinel; on Keychain failure the cleartext key is kept as a fallback.
        let db_value = crate::secrets::db_value_for_save(
            crate::secrets::store(),
            &crate::secrets::llm_account(provider),
            api_key,
        );

        let query = format!(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, "{}")
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', $1)
            ON CONFLICT(id) DO UPDATE SET
                "{}" = $1
            "#,
            api_key_column, api_key_column
        );
        sqlx::query(&query).bind(&db_value).execute(pool).await?;

        Ok(())
    }

    pub async fn get_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        // Custom OpenAI uses JSON config - extract API key from there
        if provider == "custom-openai" {
            let config = Self::get_custom_openai_config(pool).await?;
            return Ok(config.and_then(|c| c.api_key));
        }

        let api_key_column = match provider {
            "openai" => "openaiApiKey",
            "ollama" => "ollamaApiKey",
            "groq" => "groqApiKey",
            "claude" => "anthropicApiKey",
            "openrouter" => "openRouterApiKey",
            "builtin-ai" => return Ok(None), // No API key needed
            _ => {
                return Err(sqlx::Error::Protocol(format!(
                    "Invalid provider: {}",
                    provider
                )))
            }
        };

        let query = format!(
            "SELECT {} FROM settings WHERE id = '1' LIMIT 1",
            api_key_column
        );
        let db_value: Option<Option<String>> =
            sqlx::query_scalar(&query).fetch_optional(pool).await?;
        Ok(crate::secrets::resolve_secret(
            crate::secrets::store(),
            &crate::secrets::llm_account(provider),
            db_value.flatten(),
        ))
    }

    pub async fn get_transcript_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<TranscriptSetting>, sqlx::Error> {
        let setting =
            sqlx::query_as::<_, TranscriptSetting>("SELECT * FROM transcript_settings LIMIT 1")
                .fetch_optional(pool)
                .await?;
        Ok(setting)
    }

    pub async fn save_transcript_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO transcript_settings (id, provider, model)
            VALUES ('1', $1, $2)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model
            "#,
        )
        .bind(provider)
        .bind(model)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_transcript_api_key(
        pool: &SqlitePool,
        provider: &str,
        api_key: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Ignore masked/sentinel echoes from the frontend (ADR-0009).
        if crate::secrets::is_placeholder(api_key) {
            log::debug!(
                "save_transcript_api_key: ignoring placeholder value for provider '{provider}'"
            );
            return Ok(());
        }

        let api_key_column = match provider {
            "localWhisper" => "whisperApiKey",
            "parakeet" => return Ok(()), // Parakeet doesn't need an API key, return early
            "deepgram" => "deepgramApiKey",
            "elevenLabs" => "elevenLabsApiKey",
            "groq" => "groqApiKey",
            "openai" => "openaiApiKey",
            _ => {
                return Err(sqlx::Error::Protocol(format!(
                    "Invalid provider: {}",
                    provider
                )))
            }
        };

        // Keychain-first (ADR-0009), cleartext DB value only as fallback.
        let db_value = crate::secrets::db_value_for_save(
            crate::secrets::store(),
            &crate::secrets::transcript_account(provider),
            api_key,
        );

        let query = format!(
            r#"
            INSERT INTO transcript_settings (id, provider, model, "{}")
            VALUES ('1', 'parakeet', '{}', $1)
            ON CONFLICT(id) DO UPDATE SET
                "{}" = $1
            "#,
            api_key_column,
            crate::config::DEFAULT_PARAKEET_MODEL,
            api_key_column
        );
        sqlx::query(&query).bind(&db_value).execute(pool).await?;

        Ok(())
    }

    pub async fn get_transcript_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        let api_key_column = match provider {
            "localWhisper" => "whisperApiKey",
            "parakeet" => return Ok(None), // Parakeet doesn't need an API key
            "deepgram" => "deepgramApiKey",
            "elevenLabs" => "elevenLabsApiKey",
            "groq" => "groqApiKey",
            "openai" => "openaiApiKey",
            _ => {
                return Err(sqlx::Error::Protocol(format!(
                    "Invalid provider: {}",
                    provider
                )))
            }
        };

        let query = format!(
            "SELECT {} FROM transcript_settings WHERE id = '1' LIMIT 1",
            api_key_column
        );
        let db_value: Option<Option<String>> =
            sqlx::query_scalar(&query).fetch_optional(pool).await?;
        Ok(crate::secrets::resolve_secret(
            crate::secrets::store(),
            &crate::secrets::transcript_account(provider),
            db_value.flatten(),
        ))
    }

    pub async fn delete_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Custom OpenAI uses JSON config - clear the entire config
        if provider == "custom-openai" {
            sqlx::query("UPDATE settings SET customOpenAIConfig = NULL WHERE id = '1'")
                .execute(pool)
                .await?;
            crate::secrets::delete_secret(
                crate::secrets::store(),
                &crate::secrets::llm_account(provider),
            );
            return Ok(());
        }

        let api_key_column = match provider {
            "openai" => "openaiApiKey",
            "ollama" => "ollamaApiKey",
            "groq" => "groqApiKey",
            "claude" => "anthropicApiKey",
            "openrouter" => "openRouterApiKey",
            "builtin-ai" => return Ok(()), // No API key needed
            _ => {
                return Err(sqlx::Error::Protocol(format!(
                    "Invalid provider: {}",
                    provider
                )))
            }
        };

        let query = format!(
            "UPDATE settings SET {} = NULL WHERE id = '1'",
            api_key_column
        );
        sqlx::query(&query).execute(pool).await?;
        crate::secrets::delete_secret(
            crate::secrets::store(),
            &crate::secrets::llm_account(provider),
        );

        Ok(())
    }

    // ===== CUSTOM OPENAI CONFIG METHODS =====

    /// Gets the custom OpenAI configuration from JSON, with the API key resolved
    /// through the Keychain (ADR-0009): a sentinel `api_key` becomes the real key
    /// when the Keychain item is readable, `None` (endpoint treated as key-less)
    /// otherwise.
    ///
    /// # Returns
    /// * `Ok(Some(CustomOpenAIConfig))` - Config exists and is valid JSON
    /// * `Ok(None)` - No config stored
    /// * `Err(sqlx::Error)` - Database error
    pub async fn get_custom_openai_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<CustomOpenAIConfig>, sqlx::Error> {
        let mut config = Self::get_custom_openai_config_raw(pool).await?;
        if let Some(cfg) = config.as_mut() {
            cfg.api_key = crate::secrets::resolve_secret(
                crate::secrets::store(),
                &crate::secrets::llm_account("custom-openai"),
                cfg.api_key.take(),
            );
        }
        Ok(config)
    }

    /// The stored custom OpenAI config **as persisted** — `api_key` may be the
    /// Keychain sentinel or a legacy cleartext key. Used by the save path and the
    /// startup migration; everything else should use `get_custom_openai_config`.
    async fn get_custom_openai_config_raw(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<CustomOpenAIConfig>, sqlx::Error> {
        use sqlx::Row;

        let row = sqlx::query(
            r#"
            SELECT customOpenAIConfig
            FROM settings
            WHERE id = '1'
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await?;

        match row {
            Some(record) => {
                let config_json: Option<String> = record.get("customOpenAIConfig");

                if let Some(json) = config_json {
                    // Parse JSON into CustomOpenAIConfig
                    let config: CustomOpenAIConfig = serde_json::from_str(&json).map_err(|e| {
                        sqlx::Error::Protocol(format!("Invalid JSON in customOpenAIConfig: {}", e))
                    })?;

                    Ok(Some(config))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Saves the custom OpenAI configuration as JSON
    ///
    /// # Arguments
    /// * `pool` - Database connection pool
    /// * `config` - CustomOpenAIConfig to save (includes endpoint, apiKey, model, maxTokens, temperature, topP)
    ///
    /// # Returns
    /// * `Ok(())` - Config saved successfully
    /// * `Err(sqlx::Error)` - Database or JSON serialization error
    pub async fn save_custom_openai_config(
        pool: &SqlitePool,
        config: &CustomOpenAIConfig,
    ) -> std::result::Result<(), sqlx::Error> {
        // Decide what the persisted `api_key` field should hold (ADR-0009):
        // - a real new key → Keychain-first (sentinel in the JSON on success);
        // - empty/None or a masked/sentinel echo from the write-only frontend →
        //   PRESERVE the currently stored representation, so editing the endpoint
        //   never silently wipes the key. Removing the key entirely goes through
        //   delete_api_key("custom-openai").
        let mut to_store = config.clone();
        let incoming = to_store
            .api_key
            .take()
            .filter(|k| !k.trim().is_empty() && !crate::secrets::is_placeholder(k));
        to_store.api_key = match incoming {
            Some(key) => Some(crate::secrets::db_value_for_save(
                crate::secrets::store(),
                &crate::secrets::llm_account("custom-openai"),
                &key,
            )),
            None => Self::get_custom_openai_config_raw(pool)
                .await?
                .and_then(|existing| existing.api_key),
        };
        let config = &to_store;

        // Serialize config to JSON
        let config_json = serde_json::to_string(config).map_err(|e| {
            sqlx::Error::Protocol(format!("Failed to serialize config to JSON: {}", e))
        })?;

        // Upsert into settings table
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, customOpenAIConfig)
            VALUES ('1', 'custom-openai', $1, 'large-v3', $2)
            ON CONFLICT(id) DO UPDATE SET
                customOpenAIConfig = excluded.customOpenAIConfig
            "#,
        )
        .bind(&config.model)
        .bind(config_json)
        .execute(pool)
        .await?;

        Ok(())
    }

    // ===== KEYCHAIN MIGRATION (spec 0030 WS1 / ADR-0009) =====

    /// One-time (idempotent) startup migration: move every cleartext API key out
    /// of SQLite into the secret store. Per key: write to the store; on success
    /// replace the DB value with the Keychain sentinel; on ANY store failure leave
    /// the DB value untouched and log a warning (it stays readable via the DB
    /// fallback and is retried on the next launch). Sentinel/empty values are
    /// skipped, so running this on every startup is cheap and safe.
    pub async fn migrate_keys_to_keychain(
        pool: &SqlitePool,
        store: &dyn crate::secrets::SecretStore,
    ) -> std::result::Result<(), sqlx::Error> {
        use crate::secrets::{
            db_value_for_save, is_placeholder, llm_account, transcript_account, KEYCHAIN_SENTINEL,
        };

        let mut migrated = 0usize;

        // -- settings table (summarization providers) ------------------------
        if let Some(setting) = Self::get_model_config(pool).await? {
            let columns = [
                ("openaiApiKey", "openai", &setting.openai_api_key),
                ("anthropicApiKey", "claude", &setting.anthropic_api_key),
                ("ollamaApiKey", "ollama", &setting.ollama_api_key),
                ("groqApiKey", "groq", &setting.groq_api_key),
                (
                    "openRouterApiKey",
                    "openrouter",
                    &setting.open_router_api_key,
                ),
            ];
            for (column, provider, value) in columns {
                let Some(key) = value
                    .as_deref()
                    .filter(|k| !k.is_empty() && !is_placeholder(k))
                else {
                    continue;
                };
                let db_value = db_value_for_save(Some(store), &llm_account(provider), key);
                if db_value == KEYCHAIN_SENTINEL {
                    let query = format!("UPDATE settings SET \"{column}\" = $1 WHERE id = '1'");
                    sqlx::query(&query).bind(&db_value).execute(pool).await?;
                    migrated += 1;
                }
                // On store failure db_value_for_save already logged a warning and
                // the cleartext column is left untouched for the next launch.
            }
        }

        // -- custom-openai key nested in the JSON blob ------------------------
        if let Some(raw) = Self::get_custom_openai_config_raw(pool).await? {
            if let Some(key) = raw
                .api_key
                .as_deref()
                .filter(|k| !k.is_empty() && !is_placeholder(k))
            {
                let db_value = db_value_for_save(Some(store), &llm_account("custom-openai"), key);
                if db_value == KEYCHAIN_SENTINEL {
                    let mut updated = raw.clone();
                    updated.api_key = Some(db_value);
                    let json = serde_json::to_string(&updated).map_err(|e| {
                        sqlx::Error::Protocol(format!(
                            "Failed to re-serialize customOpenAIConfig during key migration: {e}"
                        ))
                    })?;
                    sqlx::query("UPDATE settings SET customOpenAIConfig = $1 WHERE id = '1'")
                        .bind(json)
                        .execute(pool)
                        .await?;
                    migrated += 1;
                }
            }
        }

        // -- transcript_settings table (transcription providers) --------------
        if let Some(setting) = Self::get_transcript_config(pool).await? {
            let columns = [
                ("whisperApiKey", "localWhisper", &setting.whisper_api_key),
                ("deepgramApiKey", "deepgram", &setting.deepgram_api_key),
                (
                    "elevenLabsApiKey",
                    "elevenLabs",
                    &setting.eleven_labs_api_key,
                ),
                ("groqApiKey", "groq", &setting.groq_api_key),
                ("openaiApiKey", "openai", &setting.openai_api_key),
            ];
            for (column, provider, value) in columns {
                let Some(key) = value
                    .as_deref()
                    .filter(|k| !k.is_empty() && !is_placeholder(k))
                else {
                    continue;
                };
                let db_value = db_value_for_save(Some(store), &transcript_account(provider), key);
                if db_value == KEYCHAIN_SENTINEL {
                    let query =
                        format!("UPDATE transcript_settings SET \"{column}\" = $1 WHERE id = '1'");
                    sqlx::query(&query).bind(&db_value).execute(pool).await?;
                    migrated += 1;
                }
            }
        }

        if migrated > 0 {
            log::info!("Migrated {migrated} API key(s) from SQLite to the Keychain");
        }
        Ok(())
    }

    /// Run the Keychain migration against the global store, if one is configured.
    /// Never fails the caller: DB errors are logged and swallowed (the keys stay
    /// readable via the cleartext fallback).
    pub async fn migrate_keys_to_keychain_best_effort(pool: &SqlitePool) {
        let Some(store) = crate::secrets::store() else {
            log::debug!("Keychain migration skipped: no secret store configured");
            return;
        };
        if let Err(e) = Self::migrate_keys_to_keychain(pool, store).await {
            log::warn!(
                "API-key Keychain migration failed (keys remain readable from the database \
                 this launch, will retry next start): {e}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemoryStore, SecretStore, KEYCHAIN_SENTINEL};
    use sqlx::sqlite::SqlitePoolOptions;

    /// Minimal schema clone of the two settings tables (column names must match
    /// production; see migrations).
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            r#"
            CREATE TABLE settings (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                whisperModel TEXT NOT NULL,
                groqApiKey TEXT,
                openaiApiKey TEXT,
                anthropicApiKey TEXT,
                ollamaApiKey TEXT,
                openRouterApiKey TEXT,
                ollamaEndpoint TEXT,
                customOpenAIConfig TEXT
            );
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            CREATE TABLE transcript_settings (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                whisperApiKey TEXT,
                deepgramApiKey TEXT,
                elevenLabsApiKey TEXT,
                groqApiKey TEXT,
                openaiApiKey TEXT
            );
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn migration_moves_cleartext_keys_and_writes_sentinel() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel,
                                  anthropicApiKey, groqApiKey, customOpenAIConfig)
            VALUES ('1', 'claude', 'claude-sonnet-4-5', 'large-v3',
                    'sk-ant-secret', 'gsk-secret',
                    '{"endpoint":"http://localhost:8000/v1","apiKey":"custom-secret","model":"m","maxTokens":null,"temperature":null,"topP":null}')
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO transcript_settings (id, provider, model, deepgramApiKey)
             VALUES ('1', 'parakeet', 'p', 'dg-secret')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = MemoryStore::new();
        SettingsRepository::migrate_keys_to_keychain(&pool, &store)
            .await
            .unwrap();

        // Keys landed in the store, namespaced per table.
        assert_eq!(
            store.get("llm.claude").unwrap().as_deref(),
            Some("sk-ant-secret")
        );
        assert_eq!(
            store.get("llm.groq").unwrap().as_deref(),
            Some("gsk-secret")
        );
        assert_eq!(
            store.get("llm.custom-openai").unwrap().as_deref(),
            Some("custom-secret")
        );
        assert_eq!(
            store.get("transcript.deepgram").unwrap().as_deref(),
            Some("dg-secret")
        );

        // DB columns now hold only the sentinel.
        let (anthropic, groq, custom_json): (Option<String>, Option<String>, Option<String>) =
            sqlx::query_as(
                "SELECT anthropicApiKey, groqApiKey, customOpenAIConfig FROM settings WHERE id='1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(anthropic.as_deref(), Some(KEYCHAIN_SENTINEL));
        assert_eq!(groq.as_deref(), Some(KEYCHAIN_SENTINEL));
        assert!(custom_json.unwrap().contains(KEYCHAIN_SENTINEL));
        let dg: Option<String> =
            sqlx::query_scalar("SELECT deepgramApiKey FROM transcript_settings WHERE id='1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(dg.as_deref(), Some(KEYCHAIN_SENTINEL));

        // Idempotent: a second run is a no-op.
        SettingsRepository::migrate_keys_to_keychain(&pool, &store)
            .await
            .unwrap();
        assert_eq!(
            store.get("llm.claude").unwrap().as_deref(),
            Some("sk-ant-secret")
        );
    }

    #[tokio::test]
    async fn migration_leaves_db_untouched_when_store_fails() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO settings (id, provider, model, whisperModel, openaiApiKey)
             VALUES ('1', 'openai', 'gpt-4o', 'large-v3', 'sk-cleartext')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let store = MemoryStore::new();
        store.fail.store(true, std::sync::atomic::Ordering::Relaxed);
        SettingsRepository::migrate_keys_to_keychain(&pool, &store)
            .await
            .unwrap();

        let openai: Option<String> =
            sqlx::query_scalar("SELECT openaiApiKey FROM settings WHERE id='1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(openai.as_deref(), Some("sk-cleartext"));
    }

    #[tokio::test]
    async fn placeholder_saves_never_clobber_columns() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO settings (id, provider, model, whisperModel, groqApiKey)
             VALUES ('1', 'groq', 'llama', 'large-v3', 'gsk-real')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // A masked hint echoed by the frontend must be ignored.
        SettingsRepository::save_api_key(&pool, "groq", "••••1234")
            .await
            .unwrap();
        SettingsRepository::save_api_key(&pool, "groq", KEYCHAIN_SENTINEL)
            .await
            .unwrap();

        let groq: Option<String> =
            sqlx::query_scalar("SELECT groqApiKey FROM settings WHERE id='1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(groq.as_deref(), Some("gsk-real"));
    }

    #[tokio::test]
    async fn saving_custom_config_without_key_preserves_existing_key() {
        let pool = test_pool().await;
        let initial = CustomOpenAIConfig {
            endpoint: "http://localhost:8000/v1".to_string(),
            api_key: Some("real-key".to_string()),
            model: "m1".to_string(),
            max_tokens: None,
            temperature: None,
            top_p: None,
        };
        SettingsRepository::save_custom_openai_config(&pool, &initial)
            .await
            .unwrap();

        // Re-save with a changed endpoint and no key (write-only frontend field
        // left blank): the stored key representation must survive.
        let updated = CustomOpenAIConfig {
            endpoint: "http://localhost:9000/v1".to_string(),
            api_key: None,
            model: "m2".to_string(),
            max_tokens: Some(2048),
            temperature: None,
            top_p: None,
        };
        SettingsRepository::save_custom_openai_config(&pool, &updated)
            .await
            .unwrap();

        let stored = SettingsRepository::get_custom_openai_config(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.endpoint, "http://localhost:9000/v1");
        assert_eq!(stored.api_key.as_deref(), Some("real-key"));
    }
}
