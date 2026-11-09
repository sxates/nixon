//! The agent access log: one row per MCP call, in its own SQLite file
//! (`agent_access_log.sqlite`) so the MCP process and the app can both write it without
//! touching the meetings database. WAL + a 5 s busy timeout make concurrent writers safe.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessEntry {
    /// RFC 3339, UTC.
    pub at: String,
    pub client: Option<String>,
    pub tool: String,
    pub args: serde_json::Value,
    pub meeting_ids: Vec<String>,
    /// "ok" | "off" | "needs_update" | "no_db" | "error"
    pub outcome: String,
}

pub struct AccessLog {
    pool: SqlitePool,
}

impl AccessLog {
    pub async fn open(data_dir: &Path) -> Result<AccessLog> {
        std::fs::create_dir_all(data_dir).context("Could not create Nixon's data folder")?;
        let opts = SqliteConnectOptions::new()
            .filename(data_dir.join("agent_access_log.sqlite"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(opts)
            .await
            .context("Could not open the agent access log")?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS access_log (
                id INTEGER PRIMARY KEY,
                at TEXT NOT NULL,
                client TEXT,
                tool TEXT NOT NULL,
                args_json TEXT NOT NULL,
                meeting_ids_json TEXT NOT NULL,
                outcome TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_access_log_at ON access_log(at)")
            .execute(&pool)
            .await?;
        Ok(AccessLog { pool })
    }

    pub async fn record(&self, e: &AccessEntry) -> Result<()> {
        sqlx::query(
            "INSERT INTO access_log (at, client, tool, args_json, meeting_ids_json, outcome)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&e.at)
        .bind(&e.client)
        .bind(&e.tool)
        .bind(serde_json::to_string(&e.args)?)
        .bind(serde_json::to_string(&e.meeting_ids)?)
        .bind(&e.outcome)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Deletes entries older than `days` days; returns how many were removed.
    pub async fn prune_older_than_days(&self, days: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let r = sqlx::query("DELETE FROM access_log WHERE at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Newest first.
    pub async fn list(&self, limit: u32) -> Result<Vec<AccessEntry>> {
        let rows = sqlx::query(
            "SELECT at, client, tool, args_json, meeting_ids_json, outcome
             FROM access_log ORDER BY at DESC, id DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(AccessEntry {
                    at: r.get("at"),
                    client: r.get("client"),
                    tool: r.get("tool"),
                    args: serde_json::from_str(&r.get::<String, _>("args_json"))
                        .unwrap_or(serde_json::Value::Null),
                    meeting_ids: serde_json::from_str(&r.get::<String, _>("meeting_ids_json"))
                        .unwrap_or_default(),
                    outcome: r.get("outcome"),
                })
            })
            .collect()
    }

    pub async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM access_log")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as D, SecondsFormat, Utc};

    fn entry(at: chrono::DateTime<Utc>, tool: &str) -> AccessEntry {
        AccessEntry {
            at: at.to_rfc3339_opts(SecondsFormat::Millis, true),
            client: Some("test-client".into()),
            tool: tool.into(),
            args: serde_json::json!({"q": "x"}),
            meeting_ids: vec!["m1".into()],
            outcome: "ok".into(),
        }
    }

    #[tokio::test]
    async fn records_and_lists_newest_first() {
        let d = tempfile::tempdir().unwrap();
        let log = AccessLog::open(d.path()).await.unwrap();
        let now = Utc::now();
        log.record(&entry(now - D::minutes(5), "older"))
            .await
            .unwrap();
        log.record(&entry(now, "newer")).await.unwrap();
        let l = log.list(10).await.unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].tool, "newer");
        assert_eq!(l[1].meeting_ids, vec!["m1".to_string()]);
        assert_eq!(l[1].args["q"], "x");
        assert_eq!(log.list(1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn prunes_entries_older_than_90_days() {
        let d = tempfile::tempdir().unwrap();
        let log = AccessLog::open(d.path()).await.unwrap();
        let now = Utc::now();
        log.record(&entry(now - D::days(91), "old")).await.unwrap();
        log.record(&entry(now - D::days(89), "recent"))
            .await
            .unwrap();
        assert_eq!(log.prune_older_than_days(90).await.unwrap(), 1);
        let l = log.list(10).await.unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].tool, "recent");
    }

    #[tokio::test]
    async fn clear_empties() {
        let d = tempfile::tempdir().unwrap();
        let log = AccessLog::open(d.path()).await.unwrap();
        log.record(&entry(Utc::now(), "a")).await.unwrap();
        log.clear().await.unwrap();
        assert!(log.list(10).await.unwrap().is_empty());
    }

    async fn write_many(log: &AccessLog, tag: &str, now: chrono::DateTime<Utc>) {
        for i in 0..50 {
            log.record(&entry(now, &format!("{tag}{i}"))).await.unwrap();
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn two_processes_write_concurrently() {
        let d = tempfile::tempdir().unwrap();
        let a = AccessLog::open(d.path()).await.unwrap();
        let b = AccessLog::open(d.path()).await.unwrap();
        let now = Utc::now();
        tokio::join!(write_many(&a, "a", now), write_many(&b, "b", now));
        assert_eq!(a.list(500).await.unwrap().len(), 100);
    }
}
