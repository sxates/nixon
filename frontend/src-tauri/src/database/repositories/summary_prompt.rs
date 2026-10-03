//! Saved summary prompts (specs/0079): the free-form replacement for summary templates.
//!
//! `create`/`update` take ALREADY-sanitized name/body (callers sanitize); the repository
//! enforces name uniqueness (case-insensitive, library prompts only) via a partial index.

use anyhow::{anyhow, Context, Result};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, serde::Serialize)]
pub struct SummaryPrompt {
    pub id: String,
    pub name: String,
    pub body: String,
    pub sort_order: i64,
    pub is_default: bool,
    pub extract_action_items: bool,
    pub in_library: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeetingPromptFields {
    pub summary_prompt_id: Option<String>,
    pub custom_summary_prompt: Option<String>,
    pub custom_extract_action_items: Option<bool>,
    pub calendar_series_key: Option<String>,
    pub title: String,
}

pub struct SummaryPromptRepository;

type MeetingFieldsRow = (
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    String,
);

pub fn series_key_for(calendar_series_key: Option<&str>, title: &str) -> Option<String> {
    if let Some(k) = calendar_series_key.map(str::trim).filter(|k| !k.is_empty()) {
        return Some(format!("cal:{k}"));
    }
    let normalized = crate::database::repositories::meeting::normalize_title(title);
    if normalized.is_empty() || is_placeholder_title(title) {
        return None;
    }
    Some(format!("title:{normalized}"))
}

/// Auto-generated date-stamp titles ("Meeting 2026-…") carry no recurring-meeting signal
/// (mirrors the frontend's `isSuggestableTitle`).
fn is_placeholder_title(title: &str) -> bool {
    let t = title.trim();
    t.strip_prefix("Meeting ")
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_ascii_digit())
}

const COLS: &str = "id, name, body, sort_order, is_default, extract_action_items, \
                    in_library, created_at, updated_at";

fn map_write_err(e: sqlx::Error, name: &str) -> anyhow::Error {
    match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            anyhow!("A prompt named \"{name}\" already exists.")
        }
        other => anyhow!(other).context("Failed to save the summary prompt"),
    }
}

impl SummaryPromptRepository {
    pub async fn list(pool: &SqlitePool) -> Result<Vec<SummaryPrompt>> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM summary_prompts WHERE in_library = 1 \
             ORDER BY sort_order, created_at"
        ))
        .fetch_all(pool)
        .await?)
    }

    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<SummaryPrompt>> {
        Ok(
            sqlx::query_as(&format!("SELECT {COLS} FROM summary_prompts WHERE id = ?"))
                .bind(id)
                .fetch_optional(pool)
                .await?,
        )
    }

    pub async fn get_default(pool: &SqlitePool) -> Result<Option<SummaryPrompt>> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM summary_prompts WHERE is_default = 1 LIMIT 1"
        ))
        .fetch_optional(pool)
        .await?)
    }

    pub async fn create(
        pool: &SqlitePool,
        name: &str,
        body: &str,
        extract_action_items: bool,
        in_library: bool,
    ) -> Result<SummaryPrompt> {
        let id = format!("prompt-{}", uuid::Uuid::new_v4());
        let mut tx = pool.begin().await?;
        let has_default: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM summary_prompts WHERE is_default = 1")
                .fetch_one(&mut *tx)
                .await?;
        let next_order: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM summary_prompts")
                .fetch_one(&mut *tx)
                .await?;
        let make_default = has_default == 0 && in_library;
        sqlx::query(
            "INSERT INTO summary_prompts \
             (id, name, body, sort_order, is_default, extract_action_items, in_library) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(body)
        .bind(next_order)
        .bind(i64::from(make_default))
        .bind(i64::from(extract_action_items))
        .bind(i64::from(in_library))
        .execute(&mut *tx)
        .await
        .map_err(|e| map_write_err(e, name))?;
        tx.commit().await?;
        Self::get(pool, &id)
            .await?
            .context("prompt vanished after insert")
    }

    pub async fn update(
        pool: &SqlitePool,
        id: &str,
        name: &str,
        body: &str,
        extract_action_items: bool,
    ) -> Result<Option<SummaryPrompt>> {
        let res = sqlx::query(
            "UPDATE summary_prompts SET name = ?, body = ?, extract_action_items = ?, \
             updated_at = datetime('now') WHERE id = ?",
        )
        .bind(name)
        .bind(body)
        .bind(i64::from(extract_action_items))
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| map_write_err(e, name))?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        Self::get(pool, id).await
    }

    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
        let mut tx = pool.begin().await?;
        let is_default: Option<i64> =
            sqlx::query_scalar("SELECT is_default FROM summary_prompts WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        match is_default {
            None => return Ok(()),
            Some(1) => {
                return Err(anyhow!(
                    "You can't delete the default prompt. Choose another default first."
                ))
            }
            Some(_) => {}
        }
        sqlx::query("DELETE FROM series_summary_prompts WHERE prompt_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM summary_prompts WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_default(pool: &SqlitePool, id: &str) -> Result<bool> {
        let mut tx = pool.begin().await?;
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM summary_prompts WHERE id = ? AND in_library = 1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if exists.is_none() {
            return Ok(false);
        }
        sqlx::query("UPDATE summary_prompts SET is_default = 0 WHERE is_default = 1")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE summary_prompts SET is_default = 1, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn get_series_prompt(
        pool: &SqlitePool,
        series_key: &str,
    ) -> Result<Option<SummaryPrompt>> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM summary_prompts WHERE id = \
             (SELECT prompt_id FROM series_summary_prompts WHERE series_key = ?)"
        ))
        .bind(series_key)
        .fetch_optional(pool)
        .await?)
    }

    pub async fn set_series_prompt(
        pool: &SqlitePool,
        series_key: &str,
        prompt_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO series_summary_prompts (series_key, prompt_id) VALUES (?, ?) \
             ON CONFLICT(series_key) DO UPDATE SET prompt_id = excluded.prompt_id, \
             updated_at = datetime('now')",
        )
        .bind(series_key)
        .bind(prompt_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn meeting_fields(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingPromptFields>> {
        let row: Option<MeetingFieldsRow> = sqlx::query_as(
            "SELECT summary_prompt_id, custom_summary_prompt, custom_extract_action_items, \
                    calendar_series_key, title FROM meetings WHERE id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|(p, c, x, k, t)| MeetingPromptFields {
            summary_prompt_id: p,
            custom_summary_prompt: c,
            custom_extract_action_items: x.map(|v| v != 0),
            calendar_series_key: k,
            title: t,
        }))
    }

    /// Metadata write: deliberately does not bump `meetings.updated_at`.
    pub async fn set_meeting_prompt_id(
        pool: &SqlitePool,
        meeting_id: &str,
        prompt_id: Option<&str>,
    ) -> Result<bool> {
        let res = sqlx::query("UPDATE meetings SET summary_prompt_id = ? WHERE id = ?")
            .bind(prompt_id)
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Empty/absent body clears both the body and the extract flag (NULL). Does not bump
    /// `meetings.updated_at`.
    pub async fn set_meeting_custom_prompt(
        pool: &SqlitePool,
        meeting_id: &str,
        body: Option<&str>,
        extract: bool,
    ) -> Result<bool> {
        let body = body.map(str::trim).filter(|b| !b.is_empty());
        let flag = body.map(|_| i64::from(extract));
        let res = sqlx::query(
            "UPDATE meetings SET custom_summary_prompt = ?, custom_extract_action_items = ? \
             WHERE id = ?",
        )
        .bind(body)
        .bind(flag)
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_meta(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT value FROM summary_prompts_meta WHERE key = ?")
                .bind(key)
                .fetch_optional(pool)
                .await?,
        )
    }

    pub async fn set_meta(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO summary_prompts_meta (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::memory_db;
    use crate::database::repositories::meeting::MeetingsRepository;
    use chrono::Utc;

    #[tokio::test]
    async fn first_prompt_becomes_default_and_exactly_one_default_exists() {
        let pool = memory_db().await;
        let a = SummaryPromptRepository::create(&pool, "A", "body a", true, true)
            .await
            .unwrap();
        let b = SummaryPromptRepository::create(&pool, "B", "body b", true, true)
            .await
            .unwrap();
        assert!(a.is_default);
        assert!(!b.is_default);
        assert!(SummaryPromptRepository::set_default(&pool, &b.id)
            .await
            .unwrap());
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM summary_prompts WHERE is_default = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            SummaryPromptRepository::get_default(&pool)
                .await
                .unwrap()
                .unwrap()
                .id,
            b.id
        );
    }

    #[tokio::test]
    async fn delete_refuses_the_default_but_removes_others_and_their_series_rows() {
        let pool = memory_db().await;
        let a = SummaryPromptRepository::create(&pool, "A", "a", true, true)
            .await
            .unwrap();
        let b = SummaryPromptRepository::create(&pool, "B", "b", true, true)
            .await
            .unwrap();
        SummaryPromptRepository::set_series_prompt(&pool, "cal:k", &b.id)
            .await
            .unwrap();
        let err = SummaryPromptRepository::delete(&pool, &a.id)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("default"), "{err}");
        SummaryPromptRepository::delete(&pool, &b.id).await.unwrap();
        assert!(SummaryPromptRepository::get_series_prompt(&pool, "cal:k")
            .await
            .unwrap()
            .is_none());
        assert!(SummaryPromptRepository::get(&pool, &b.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn names_are_unique_case_insensitively_among_library_prompts() {
        let pool = memory_db().await;
        SummaryPromptRepository::create(&pool, "Standup", "x", true, true)
            .await
            .unwrap();
        let err = SummaryPromptRepository::create(&pool, "standup", "y", true, true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        SummaryPromptRepository::create(&pool, "standup", "y", true, false)
            .await
            .expect("series-only prompt may reuse a library name");
    }

    #[tokio::test]
    async fn list_excludes_series_only_prompts_but_get_finds_them() {
        let pool = memory_db().await;
        SummaryPromptRepository::create(&pool, "Lib", "x", true, true)
            .await
            .unwrap();
        let hidden = SummaryPromptRepository::create(&pool, "Hidden", "y", true, false)
            .await
            .unwrap();
        let listed = SummaryPromptRepository::list(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Lib");
        let got = SummaryPromptRepository::get(&pool, &hidden.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!got.in_library);
        assert!(!got.is_default);
    }

    #[tokio::test]
    async fn meeting_fields_roundtrip() {
        let pool = memory_db().await;
        let p = SummaryPromptRepository::create(&pool, "P", "x", true, true)
            .await
            .unwrap();
        let id = MeetingsRepository::create_meeting(
            &pool,
            Some("Weekly Sync".into()),
            None,
            None,
            None,
            Some(Utc::now()),
        )
        .await
        .unwrap();
        assert!(
            SummaryPromptRepository::set_meeting_prompt_id(&pool, &id, Some(&p.id))
                .await
                .unwrap()
        );
        assert!(SummaryPromptRepository::set_meeting_custom_prompt(
            &pool,
            &id,
            Some("be terse"),
            false
        )
        .await
        .unwrap());
        let f = SummaryPromptRepository::meeting_fields(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.summary_prompt_id.as_deref(), Some(p.id.as_str()));
        assert_eq!(f.custom_summary_prompt.as_deref(), Some("be terse"));
        assert_eq!(f.custom_extract_action_items, Some(false));
        assert_eq!(f.title, "Weekly Sync");

        SummaryPromptRepository::set_meeting_custom_prompt(&pool, &id, None, true)
            .await
            .unwrap();
        let f = SummaryPromptRepository::meeting_fields(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.custom_summary_prompt, None);
    }

    #[test]
    fn series_key_for_prefers_calendar_key_then_normalized_title() {
        assert_eq!(
            series_key_for(Some("abc"), "Weekly"),
            Some("cal:abc".to_string())
        );
        assert_eq!(
            series_key_for(None, "  Weekly SYNC "),
            Some("title:weekly sync".to_string())
        );
        assert_eq!(series_key_for(Some("  "), ""), None);
        assert_eq!(series_key_for(None, "Meeting 2026-10-03 09:00"), None);
    }
}
