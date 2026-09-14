use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tauri::command;

/// Groq model information returned to frontend
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GroqModel {
    pub id: String,
    pub owned_by: Option<String>,
}

/// API response model from Groq (OpenAI-compatible format)
#[derive(Debug, Deserialize)]
struct GroqApiModel {
    id: String,
    owned_by: Option<String>,
    #[allow(dead_code)]
    object: String,
}

/// API response wrapper from Groq
#[derive(Debug, Deserialize)]
struct GroqApiResponse {
    data: Vec<GroqApiModel>,
}

/// Cache entry for models
struct CacheEntry {
    models: Vec<GroqModel>,
    fetched_at: Instant,
}

/// Global cache for Groq models (5 minute TTL)
static MODELS_CACHE: RwLock<Option<CacheEntry>> = RwLock::new(None);

/// Cache TTL in seconds
const CACHE_TTL_SECS: u64 = 300;

/// Fallback models when API fetch fails (matches frontend hardcoded values)
const FALLBACK_MODELS: &[&str] = &["llama-3.3-70b-versatile"];

/// Get fallback models as GroqModel vec
fn get_fallback_models() -> Vec<GroqModel> {
    FALLBACK_MODELS
        .iter()
        .map(|id| GroqModel {
            id: id.to_string(),
            owned_by: None,
        })
        .collect()
}

/// Check if model is a chat-capable model (filter out whisper, etc.)
fn is_chat_model(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    // Exclude whisper, tool-use specific models, and embedding models
    !id.contains("whisper")
        && !id.contains("embed")
        && !id.contains("guard")
        && !id.contains("tool-use")
}

/// Fetch Groq models from API
///
/// # Arguments
/// * `api_key` - Groq API key
///
/// # Returns
/// Vector of available models, or fallback models on error
#[command]
pub async fn get_groq_models(
    app: tauri::AppHandle,
    api_key: Option<String>,
) -> Result<Vec<GroqModel>, String> {
    // The webview no longer holds raw keys (spec 0030 WS2): resolve a caller-
    // supplied key, else the stored key; fall back to the static list otherwise.
    let api_key = match crate::settings::resolve_provider_key(&app, "groq", api_key).await {
        Some(key) => key,
        None => {
            log::info!("No Groq API key configured, returning fallback models");
            return Ok(get_fallback_models());
        }
    };

    // Check cache first
    {
        let cache = MODELS_CACHE.read().map_err(|e| e.to_string())?;
        if let Some(entry) = cache.as_ref() {
            if entry.fetched_at.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
                log::info!(
                    "Returning cached Groq models ({} models)",
                    entry.models.len()
                );
                return Ok(entry.models.clone());
            }
        }
    }

    // Fetch from API
    log::info!("Fetching Groq models from API...");
    let client = crate::config::shared_http_client();

    let response = match client
        .get("https://api.groq.com/openai/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            log::warn!("Failed to fetch Groq models: {}. Using fallback.", e);
            return Ok(get_fallback_models());
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        log::warn!(
            "Groq API returned status {}. Using fallback models.",
            status
        );
        return Ok(get_fallback_models());
    }

    let api_response: GroqApiResponse = match response.json().await {
        Ok(data) => data,
        Err(e) => {
            log::warn!("Failed to parse Groq response: {}. Using fallback.", e);
            return Ok(get_fallback_models());
        }
    };

    // Filter to only chat models and map to our struct
    let models: Vec<GroqModel> = api_response
        .data
        .into_iter()
        .filter(|m| is_chat_model(&m.id))
        .map(|m| GroqModel {
            id: m.id,
            owned_by: m.owned_by,
        })
        .collect();

    // If no models returned, use fallback
    if models.is_empty() {
        log::warn!("No chat models returned from Groq API. Using fallback.");
        return Ok(get_fallback_models());
    }

    log::info!("Fetched {} Groq models from API", models.len());

    // Update cache
    {
        let mut cache = MODELS_CACHE.write().map_err(|e| e.to_string())?;
        *cache = Some(CacheEntry {
            models: models.clone(),
            fetched_at: Instant::now(),
        });
    }

    Ok(models)
}

/// Clear the models cache (useful when API key changes)
pub fn clear_cache() {
    if let Ok(mut cache) = MODELS_CACHE.write() {
        *cache = None;
        log::info!("Groq models cache cleared");
    }
}
