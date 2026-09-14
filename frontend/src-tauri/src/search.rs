// Full-text meeting search (specs/0033): the ranked-search command + its hit
// DTO, moved from api/api.rs (specs/0042 WS3).

use log::{error as log_error, info as log_info};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::{database::repositories::search::SearchRepository, state::AppState};

/// One ranked full-text search hit (specs/0033), across transcripts, summaries,
/// and notes. `snippet` is plain text with `\u{1}`/`\u{2}` sentinel characters
/// around matched tokens — the frontend converts sentinel spans to `<mark>` and
/// must never interpret the snippet as HTML.
///
/// `FromRow` decodes SEARCH_SQL (repositories/search.rs) by COLUMN NAME, so the
/// query's output columns must match these snake_case field names exactly.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSearchHit {
    pub meeting_id: String,
    /// Joined from `meetings` at query time.
    pub title: String,
    pub created_at: String,
    /// "transcript" | "summary" | "notes".
    pub source: String,
    /// Sentinel-delimited excerpt (see struct docs).
    pub snippet: String,
    /// Best-matching segment id — transcript hits only; used for deep-link
    /// scroll. Best-effort: "Transcribe now" regenerates segment ids.
    pub transcript_id: Option<String>,
    /// bm25 rank (lower = better). Not comparable across sources; v1 accepts this.
    pub rank: f64,
}

/// Ranked FTS5 full-text search across transcripts, summaries, and notes
/// (specs/0033). Replaces the removed `api_search_transcripts` LIKE path.
/// Returns at most `limit` (default 20) hits, best hit per (meeting, source).
#[tauri::command]
pub async fn api_search_meetings<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<MeetingSearchHit>, String> {
    let pool = state.db_manager.pool();

    match SearchRepository::search(pool, &query, limit).await {
        Ok(hits) => {
            log_info!(
                "api_search_meetings: {} hit(s) for a {}-char query",
                hits.len(),
                query.chars().count()
            );
            Ok(hits)
        }
        Err(e) => {
            log_error!("api_search_meetings failed for query '{}': {}", query, e);
            Err(format!(
                "Search failed: {}. If this persists, the search index may need a rebuild (restart the app to re-run migrations).",
                e
            ))
        }
    }
}
