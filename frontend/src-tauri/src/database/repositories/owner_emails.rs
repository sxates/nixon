//! `owner_emails` table access — the source of truth for "which invite addresses are ME
//! (the device owner)" (specs/0018 Phase A Task 1).
//!
//! EventKit's per-attendee `isCurrentUser()` only flags the calendar account's own
//! address, so aliases / personal / delegated addresses are missed and get seeded as
//! ordinary participants. A row here says "this (normalized) address resolves to the owner
//! person ([`OWNER_PERSON_ID`])". Multi-valued by design (work + personal + aliases).
//!
//! Emails are stored NORMALIZED ([`normalize_email`]: lowercased + trimmed) so the PK does
//! the dedupe and every "is this me by address?" comparison is byte-consistent. The owner
//! person keeps `email = NULL`; THIS table is the multi-email home (so it never collides
//! under people's partial-unique `idx_people_email`).
//!
//! Mirrors the sibling repos: returns `SqlxError`; the command layer maps to user-facing
//! strings. No FK to `people` — the owner is the well-known [`OWNER_PERSON_ID`] and may not
//! exist yet when the first email is added (created lazily). [`OwnerEmailsRepository::add`]
//! ensures the owner person and runs the backfill sweep.

use chrono::Utc;
use sqlx::{Error as SqlxError, SqlitePool};

use crate::database::repositories::people::PeopleRepository;
use crate::people::enroll::{ensure_owner_person, OWNER_PERSON_ID};

/// The single normalization point for owner-email comparison/storage: trim + lowercase.
/// Returns an empty string for an empty/whitespace input — callers treat empty as "no
/// email" (the `add`/`contains` paths guard on it). Lowercase+trim only — no plus-address
/// or unicode-domain canonicalization (sufficient for invite matching; `get_by_email` is
/// already COLLATE NOCASE).
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub struct OwnerEmailsRepository;

impl OwnerEmailsRepository {
    /// All owner emails (normalized), ordered — for Settings and the "is this me?"
    /// membership set used while seeding.
    pub async fn list(pool: &SqlitePool) -> Result<Vec<String>, SqlxError> {
        let rows =
            sqlx::query_scalar::<_, String>("SELECT email FROM owner_emails ORDER BY email ASC")
                .fetch_all(pool)
                .await?;
        Ok(rows)
    }

    /// Whether the (normalized) email is an owner address — the "is this me by address?"
    /// primitive. An empty/whitespace email is never an owner email.
    pub async fn contains(pool: &SqlitePool, email: &str) -> Result<bool, SqlxError> {
        let email = normalize_email(email);
        if email.is_empty() {
            return Ok(false);
        }
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM owner_emails WHERE email = ? LIMIT 1")
                .bind(&email)
                .fetch_optional(pool)
                .await?;
        Ok(found.is_some())
    }

    /// Record a (normalized) email as "me" and run the backfill sweep.
    ///
    /// `INSERT OR IGNORE` the normalized email (the PK dedupes), then — whether or not the
    /// row was new — fold any already-rostered person carrying this email into the owner
    /// (so declaring an alias in Settings retroactively cleans rosters that already list
    /// you). Ensures the owner person exists first so a later match has a target. Returns
    /// whether a new `owner_emails` row was inserted.
    ///
    /// Sweep is idempotent: at most one `get_by_email` + one `merge_person_into_owner`. A
    /// no-op email (empty) is rejected with a clear error.
    pub async fn add(pool: &SqlitePool, email: &str) -> Result<bool, SqlxError> {
        let email = normalize_email(email);
        if email.is_empty() {
            return Err(SqlxError::Protocol(
                "owner email cannot be empty".to_string(),
            ));
        }

        // The owner person must exist so matched/swept people have a merge target.
        ensure_owner_person(pool).await?;

        let now = Utc::now().to_rfc3339();
        let res =
            sqlx::query("INSERT OR IGNORE INTO owner_emails (email, created_at) VALUES (?, ?)")
                .bind(&email)
                .bind(&now)
                .execute(pool)
                .await?;
        let inserted = res.rows_affected() > 0;

        // Backfill sweep (requirement F): fold any already-rostered person whose email
        // matches into the owner. `get_by_email` is COLLATE NOCASE, so the normalized
        // address matches case-insensitively. Run on every add (not just new inserts) so a
        // re-declared alias still converges — it's cheap and idempotent.
        if let Some(person) = PeopleRepository::get_by_email(pool, &email).await? {
            if person.id != OWNER_PERSON_ID {
                crate::people::merge::merge_person_into_owner(pool, &person.id)
                    .await
                    .map_err(|e| {
                        SqlxError::Protocol(format!("backfill merge for owner email failed: {e}"))
                    })?;
            }
        }

        Ok(inserted)
    }

    /// Remove a (normalized) owner email (the address-level inverse). Does NOT un-merge
    /// already-folded people — removing an address only stops FUTURE matching. Returns
    /// whether a row was deleted.
    pub async fn remove(pool: &SqlitePool, email: &str) -> Result<bool, SqlxError> {
        let email = normalize_email(email);
        let res = sqlx::query("DELETE FROM owner_emails WHERE email = ?")
            .bind(&email)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool through the app's REAL migration set — the `add` sweep runs
    /// `merge_person_into_owner`, whose explicit cascade touches every person-referencing
    /// table (incl. `action_items` since specs/0034), so a hand-rolled schema here would
    /// drift. One connection max — each in-memory connection is a separate database.
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[test]
    fn normalize_trims_and_lowercases() {
        assert_eq!(normalize_email("  Me@Work.com "), "me@work.com");
        assert_eq!(normalize_email("ALICE@X.COM"), "alice@x.com");
        assert_eq!(normalize_email("   "), "");
    }

    #[tokio::test]
    async fn add_is_idempotent_and_list_dedupes() {
        let pool = test_pool().await;
        assert!(OwnerEmailsRepository::add(&pool, "Me@Work.com")
            .await
            .unwrap());
        // Same address, different case/whitespace → no new row.
        assert!(!OwnerEmailsRepository::add(&pool, "me@work.com ")
            .await
            .unwrap());
        let list = OwnerEmailsRepository::list(&pool).await.unwrap();
        assert_eq!(list, vec!["me@work.com".to_string()]);
        assert!(OwnerEmailsRepository::contains(&pool, " ME@WORK.COM")
            .await
            .unwrap());
        assert!(!OwnerEmailsRepository::contains(&pool, "other@x.com")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn add_sweep_folds_existing_rostered_person() {
        let pool = test_pool().await;
        // A real meeting (speakers declares meeting_id REFERENCES meetings and the pool
        // enforces FKs), plus a person already rostered under the soon-to-be-owner email.
        let meeting_id =
            crate::database::repositories::meeting::MeetingsRepository::create_meeting(
                &pool,
                Some("Sync".into()),
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let person = PeopleRepository::create(&pool, "Me", Some("me@work.com"), None, None)
            .await
            .unwrap();
        roster_add(&pool, &meeting_id, &person.id).await;
        sqlx::query(
            "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, person_id, created_at, updated_at)
             VALUES ('s1', ?,'spk1','Speaker 1', ?, '', '')",
        )
        .bind(&meeting_id)
        .bind(&person.id)
        .execute(&pool)
        .await
        .unwrap();

        // Declaring the alias sweeps the existing person into the owner.
        OwnerEmailsRepository::add(&pool, "Me@Work.com")
            .await
            .unwrap();

        // The auto-created person is gone; its roster row gone; speaker re-pointed to owner.
        let gone: Option<String> = sqlx::query_scalar("SELECT id FROM people WHERE id = ?")
            .bind(&person.id)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert!(gone.is_none(), "swept person deleted");
        let roster: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM meeting_participants WHERE person_id = ?")
                .bind(&person.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(roster, 0, "swept person's roster rows removed");
        let owner_spk: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM speakers WHERE person_id = ?")
                .bind(OWNER_PERSON_ID)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(owner_spk, 1, "speaker re-pointed to owner");
    }

    /// Tiny helper mirroring `MeetingParticipantsRepository::add` without importing it
    /// (keeps this test file's deps minimal).
    async fn roster_add(pool: &SqlitePool, meeting_id: &str, person_id: &str) {
        sqlx::query(
            "INSERT INTO meeting_participants (meeting_id, person_id, source, created_at)
             VALUES (?, ?, 'calendar', '')",
        )
        .bind(meeting_id)
        .bind(person_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Pins the contract the Google Calendar hooks rely on (specs/0032 +
    /// 0018): `api_google_calendar_connect` and the startup backfill in
    /// `calendar::google::sync` auto-add the connected account's email via
    /// [`OwnerEmailsRepository::add`]. Reconnects and every-launch backfills
    /// must be silent no-ops — `Ok(false)`, never an error, no duplicate row —
    /// even when Google reports the address with different casing.
    #[tokio::test]
    async fn google_connect_auto_add_is_silent_noop_when_already_owner() {
        let pool = test_pool().await;
        // First connect records the account address.
        assert!(OwnerEmailsRepository::add(&pool, "acct@gmail.com")
            .await
            .unwrap());
        // Reconnect / startup backfill with the same account: no-op, no error.
        assert!(!OwnerEmailsRepository::add(&pool, "Acct@Gmail.com")
            .await
            .unwrap());
        assert!(!OwnerEmailsRepository::add(&pool, "acct@gmail.com")
            .await
            .unwrap());
        assert_eq!(
            OwnerEmailsRepository::list(&pool).await.unwrap(),
            vec!["acct@gmail.com".to_string()]
        );
    }

    #[tokio::test]
    async fn add_empty_email_is_rejected() {
        let pool = test_pool().await;
        assert!(OwnerEmailsRepository::add(&pool, "   ").await.is_err());
    }
}
