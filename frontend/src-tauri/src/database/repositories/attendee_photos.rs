//! `attendee_photos` cache access (specs/0038 WS3, ADR-0010 best-effort amendment).
//!
//! The local cache of same-org attendee profile photos, stored as base64 `data:`
//! URIs so rendering never re-hits Google (LOCAL-ONLY) and a Disconnect purge
//! ([`clear_photos`]) removes every byte. Keyed by lowercased email — the join
//! key against the wire `Attendee.email` the frontend receives.
//!
//! Mirrors the sibling repos (returns `SqlxError`; the command/sync layer maps
//! to user-facing strings / logs and degrades to initials on any error).

use std::collections::HashMap;

use sqlx::{Error as SqlxError, Row, SqlitePool};

use crate::database::repositories::owner_emails::normalize_email;

pub struct AttendeePhotosRepository;

impl AttendeePhotosRepository {
    /// One attendee's cached photo `data:` URI and `fetched_at`, or `None` on a
    /// cache miss. The email is routed through the shared [`normalize_email`] so
    /// build-side and lookup-side keys can never diverge (specs/0038 WS3).
    pub async fn get_photo(
        pool: &SqlitePool,
        email: &str,
    ) -> Result<Option<(String, String)>, SqlxError> {
        let row =
            sqlx::query("SELECT photo_data_uri, fetched_at FROM attendee_photos WHERE email = ?")
                .bind(normalize_email(email))
                .fetch_optional(pool)
                .await?;
        Ok(row.map(|r| {
            (
                r.get::<String, _>("photo_data_uri"),
                r.get::<String, _>("fetched_at"),
            )
        }))
    }

    /// Insert or replace one attendee's cached photo (last-writer-wins on the
    /// normalized email). Stamps `fetched_at` for staleness re-fetch. The key is
    /// routed through [`normalize_email`] so it matches every lookup site.
    pub async fn upsert_photo(
        pool: &SqlitePool,
        email: &str,
        data_uri: &str,
        fetched_at: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO attendee_photos (email, photo_data_uri, fetched_at) VALUES (?, ?, ?)
             ON CONFLICT(email) DO UPDATE SET
                 photo_data_uri = excluded.photo_data_uri,
                 fetched_at = excluded.fetched_at",
        )
        .bind(normalize_email(email))
        .bind(data_uri)
        .bind(fetched_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Every cached photo's `fetched_at` as a `normalized email → fetched_at`
    /// map — one bulk read so the photo-fetch pass can compute the stale/absent
    /// subset without an N+1 per-email [`get_photo`] (specs/0038 WS3). Keys are
    /// already normalized at upsert time, so they compare directly.
    pub async fn fetched_at_map(pool: &SqlitePool) -> Result<HashMap<String, String>, SqlxError> {
        let rows = sqlx::query("SELECT email, fetched_at FROM attendee_photos")
            .fetch_all(pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("email"),
                    r.get::<String, _>("fetched_at"),
                )
            })
            .collect())
    }

    /// Every cached photo as a `lowercased email → data: URI` map — the one-shot
    /// lookup the wire-attendee builder uses so it doesn't hit the DB per person.
    pub async fn all_photos(pool: &SqlitePool) -> Result<HashMap<String, String>, SqlxError> {
        let rows = sqlx::query("SELECT email, photo_data_uri FROM attendee_photos")
            .fetch_all(pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get::<String, _>("email"),
                    r.get::<String, _>("photo_data_uri"),
                )
            })
            .collect())
    }

    /// Drop the whole photo cache (Disconnect purge — no photo data survives a
    /// disconnect, ADR-0010). Idempotent.
    pub async fn clear_photos(pool: &SqlitePool) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM attendee_photos")
            .execute(pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Fresh in-memory SQLite through the app's real migration set (mirrors the
    /// sibling repos) — so this also proves the 20260709000400 migration runs.
    async fn memory_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    #[tokio::test]
    async fn upsert_get_all_and_clear_roundtrip() {
        let pool = memory_db().await;
        assert!(AttendeePhotosRepository::get_photo(&pool, "a@x.com")
            .await
            .unwrap()
            .is_none());

        AttendeePhotosRepository::upsert_photo(
            &pool,
            "A@X.com",
            "data:image/jpeg;base64,aaa",
            "2026-07-07T00:00:00+00:00",
        )
        .await
        .unwrap();
        AttendeePhotosRepository::upsert_photo(
            &pool,
            "b@x.com",
            "data:image/png;base64,bbb",
            "2026-07-07T00:00:00+00:00",
        )
        .await
        .unwrap();

        // Lookup is case-insensitive (stored + queried lowercased).
        let (uri, _fetched) = AttendeePhotosRepository::get_photo(&pool, "a@x.com")
            .await
            .unwrap()
            .expect("cached");
        assert_eq!(uri, "data:image/jpeg;base64,aaa");

        // Upsert is last-writer-wins on the lowercased email.
        AttendeePhotosRepository::upsert_photo(
            &pool,
            "a@x.com",
            "data:image/jpeg;base64,zzz",
            "2026-07-08T00:00:00+00:00",
        )
        .await
        .unwrap();
        let all = AttendeePhotosRepository::all_photos(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("a@x.com").unwrap(), "data:image/jpeg;base64,zzz");
        assert_eq!(all.get("b@x.com").unwrap(), "data:image/png;base64,bbb");

        AttendeePhotosRepository::clear_photos(&pool).await.unwrap();
        assert!(AttendeePhotosRepository::all_photos(&pool)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn fetched_at_map_returns_normalized_freshness_keys() {
        let pool = memory_db().await;
        AttendeePhotosRepository::upsert_photo(
            &pool,
            "  Priya@X.com ",
            "data:image/jpeg;base64,aaa",
            "2026-07-07T00:00:00+00:00",
        )
        .await
        .unwrap();

        let map = AttendeePhotosRepository::fetched_at_map(&pool)
            .await
            .unwrap();
        assert_eq!(map.len(), 1);
        // The key is normalized (trim + lowercase) exactly like every lookup site.
        assert_eq!(
            map.get("priya@x.com").map(String::as_str),
            Some("2026-07-07T00:00:00+00:00")
        );
    }
}
