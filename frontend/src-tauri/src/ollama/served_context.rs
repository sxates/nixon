//! Detect the context Ollama is ACTUALLY serving (specs/0052).
//!
//! `/api/show` reports `<family>.context_length` — the model's ARCHITECTURAL maximum
//! (262,144 for gemma4). What the server allocates is `OLLAMA_CONTEXT_LENGTH`, or Ollama's
//! VRAM-based default. Budgeting from the architectural max meant Nixon believed it had 4x
//! the room it had, never reached the chunking path, and let long meetings be silently
//! context-shifted.
//!
//! `/api/ps` reports the loaded runner's real allocated `context_length`, but ONLY while a
//! model is loaded — which it is not at app launch. So the reading is:
//!
//! 1. probed live (authoritative when a model is loaded),
//! 2. else read from the in-memory cache,
//! 3. else read from a small on-disk store that survives restarts.
//!
//! Step 3 exists because an in-memory-only cache made *every* cold start fall back to
//! [`CONSERVATIVE_CONTEXT_FLOOR`], budgeting the first call of each session at 8k instead of
//! the real size — a 32x under-budget that chunks long meetings needlessly. Observed live on
//! 2026-08-18: `Context budget for gemma4:26b: 8192 tokens (architectural 262144, served
//! None)`. Only a genuinely first-ever run now hits the floor.
//!
//! The store is refreshed after every successful call, when the model is guaranteed loaded,
//! so detection never forces an ~11s cold load just to probe.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Last known served context per `"{endpoint}|{model}"`. No TTL: a stale reading is far
/// better than none, and it is refreshed after every successful call.
static SERVED_CACHE: Lazy<Arc<RwLock<HashMap<String, usize>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

const DEFAULT_ENDPOINT: &str = "http://localhost:11434";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Per-identifier (ADR-0004), so dev and production never share a reading.
const STORE_FILE: &str = "ollama-context.json";

fn cache_key(model_name: &str, endpoint: Option<&str>) -> String {
    format!("{}|{}", endpoint.unwrap_or(DEFAULT_ENDPOINT), model_name)
}

fn store_path() -> PathBuf {
    crate::app_paths::app_data_dir().join(STORE_FILE)
}

/// Read the persisted readings. Any failure (missing file, corrupt JSON, bad permissions)
/// yields an empty map — a lost reading costs one conservative pass, never an error.
fn read_store_at(path: &Path) -> HashMap<String, usize> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Best-effort persist. Never surfaces an error: this is a cache, not a source of truth.
fn write_store_at(path: &Path, map: &HashMap<String, usize>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(map) {
        let _ = std::fs::write(path, json);
    }
}

/// Resolution precedence: a live probe wins, then the in-memory cache, then disk.
pub(crate) fn resolve_served(
    probed: Option<usize>,
    in_memory: Option<usize>,
    on_disk: Option<usize>,
) -> Option<usize> {
    probed.or(in_memory).or(on_disk)
}

/// Pure parser for an `/api/ps` body. `None` whenever the model isn't loaded, the field is
/// absent, or the body doesn't parse — never a guess.
pub(crate) fn parse_served_context(body: &str, model_name: &str) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("models")?.as_array()?.iter().find_map(|m| {
        let matches = m.get("name").and_then(|n| n.as_str()) == Some(model_name)
            || m.get("model").and_then(|n| n.as_str()) == Some(model_name);
        if !matches {
            return None;
        }
        m.get("context_length")
            .and_then(|c| c.as_u64())
            .map(|c| c as usize)
    })
}

async fn probe(model_name: &str, endpoint: Option<&str>) -> Option<usize> {
    let base = endpoint.unwrap_or(DEFAULT_ENDPOINT);
    let body = crate::config::shared_http_client()
        .get(format!("{base}/api/ps"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    parse_served_context(&body, model_name)
}

/// Record a reading in both caches. Disk is only rewritten when the value actually changed.
async fn remember(key: &str, ctx: usize) {
    SERVED_CACHE.write().await.insert(key.to_string(), ctx);

    let path = store_path();
    let mut on_disk = read_store_at(&path);
    if on_disk.get(key) != Some(&ctx) {
        on_disk.insert(key.to_string(), ctx);
        write_store_at(&path, &on_disk);
    }
}

/// Served context for a model: a live `/api/ps` reading when the model is loaded, else the
/// last in-memory reading, else the last persisted one, else `None` (caller applies its own
/// floor).
pub async fn served_context(model_name: &str, endpoint: Option<&str>) -> Option<usize> {
    let key = cache_key(model_name, endpoint);

    if let Some(ctx) = probe(model_name, endpoint).await {
        remember(&key, ctx).await;
        return Some(ctx);
    }

    let in_memory = SERVED_CACHE.read().await.get(&key).copied();
    let on_disk = || read_store_at(&store_path()).get(&key).copied();
    resolve_served(None, in_memory, in_memory.is_none().then(on_disk).flatten())
}

/// Re-probe and update both caches. Call after a SUCCESSFUL LLM call, when the model is
/// guaranteed loaded. Best-effort: never surfaces an error to the caller.
pub async fn refresh_served_context(model_name: &str, endpoint: Option<&str>) {
    if let Some(ctx) = probe(model_name, endpoint).await {
        remember(&cache_key(model_name, endpoint), ctx).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS_BODY: &str = r#"{
      "models": [
        {"name":"gemma4:26b","model":"gemma4:26b","size":22800000000,"context_length":65536}
      ]
    }"#;

    #[test]
    fn parses_context_length_for_the_named_model() {
        assert_eq!(parse_served_context(PS_BODY, "gemma4:26b"), Some(65536));
    }

    #[test]
    fn returns_none_when_the_model_is_not_loaded() {
        assert_eq!(parse_served_context(r#"{"models":[]}"#, "gemma4:26b"), None);
    }

    #[test]
    fn returns_none_when_a_different_model_is_loaded() {
        assert_eq!(parse_served_context(PS_BODY, "qwen3.8:27b"), None);
    }

    /// Older/newer Ollama builds may omit the field entirely. Degrade, never guess.
    #[test]
    fn returns_none_when_context_length_is_absent() {
        let body = r#"{"models":[{"name":"gemma4:26b","model":"gemma4:26b"}]}"#;
        assert_eq!(parse_served_context(body, "gemma4:26b"), None);
    }

    /// `ollama ps` reports the tag-qualified name; a bare name should still match.
    #[test]
    fn matches_on_the_model_field_too() {
        let body = r#"{"models":[{"name":"x","model":"gemma4:26b","context_length":32768}]}"#;
        assert_eq!(parse_served_context(body, "gemma4:26b"), Some(32768));
    }

    #[test]
    fn returns_none_on_malformed_json() {
        assert_eq!(parse_served_context("not json", "gemma4:26b"), None);
    }

    // --- persistence (the cold-start regression) ---------------------------------

    fn temp_store(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("nixon-0052-{name}.json"));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// The regression: an in-memory-only cache made every app launch fall back to the
    /// conservative floor, budgeting the first call of each session at 8k instead of the
    /// real served size. The reading must survive a restart.
    #[test]
    fn a_persisted_reading_survives_a_restart() {
        let path = temp_store("restart");
        let mut map = HashMap::new();
        map.insert(
            "http://localhost:11434|gemma4:26b".to_string(),
            262_144usize,
        );
        write_store_at(&path, &map);

        // Fresh process: in-memory cache is empty, disk still knows.
        let recovered = read_store_at(&path);
        assert_eq!(
            recovered.get("http://localhost:11434|gemma4:26b").copied(),
            Some(262_144)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_or_corrupt_store_is_empty_not_an_error() {
        let missing = temp_store("missing");
        assert!(read_store_at(&missing).is_empty());

        let corrupt = temp_store("corrupt");
        std::fs::write(&corrupt, "{ not json").unwrap();
        assert!(read_store_at(&corrupt).is_empty());
        let _ = std::fs::remove_file(&corrupt);
    }

    #[test]
    fn readings_are_keyed_per_endpoint_and_model() {
        let path = temp_store("keys");
        let mut map = HashMap::new();
        map.insert(
            "http://localhost:11434|gemma4:26b".to_string(),
            262_144usize,
        );
        map.insert("http://localhost:11435|gemma4:26b".to_string(), 65_536usize);
        write_store_at(&path, &map);

        let back = read_store_at(&path);
        assert_eq!(
            back.get("http://localhost:11434|gemma4:26b").copied(),
            Some(262_144)
        );
        assert_eq!(
            back.get("http://localhost:11435|gemma4:26b").copied(),
            Some(65_536)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_live_probe_beats_both_caches() {
        assert_eq!(
            resolve_served(Some(65_536), Some(8_192), Some(4_096)),
            Some(65_536)
        );
    }

    #[test]
    fn memory_beats_disk_when_the_probe_is_empty() {
        assert_eq!(
            resolve_served(None, Some(65_536), Some(4_096)),
            Some(65_536)
        );
    }

    /// The cold-start path: nothing probed, nothing in memory — disk carries the session.
    #[test]
    fn disk_carries_a_cold_start() {
        assert_eq!(resolve_served(None, None, Some(262_144)), Some(262_144));
    }

    #[test]
    fn everything_empty_stays_unknown() {
        assert_eq!(resolve_served(None, None, None), None);
    }

    #[test]
    fn the_store_file_is_named_per_identifier_app_data_dir() {
        assert!(store_path().ends_with(STORE_FILE));
    }
}
