//! `meeting_participants` table access — the durable, editable participant ROSTER for a
//! meeting (specs/0017 Phase A).
//!
//! This is NOT `speakers`. `speakers` is "who actually spoke" (diarized clusters);
//! `meeting_participants` is "who was invited / is known to be in this meeting". Each
//! participant row links a meeting to an app-wide [`Person`](super::people::Person), so a
//! participant already carries cross-meeting identity, role, and notes for free.
//!
//! `source` distinguishes `'calendar'` (seeded from the linked EventKit event) from
//! `'manual'` (hand-added on a meeting screen).
//!
//! **Owner exclusion:** the roster is the OTHER people. The device owner
//! (`Attendee::is_current_user`) is excluded at seed time — "You" is implicit (the mic
//! channel), consistent with diarization's exclude-self. So every `person_id` here is a
//! remote person, and [`remote_person_ids`](MeetingParticipantsRepository::remote_person_ids)
//! additionally hard-excludes [`OWNER_PERSON_ID`] as a belt-and-suspenders guard for the
//! diarization-cap consumer.
//!
//! Mirrors `people.rs`/`speaker.rs`: returns `SqlxError`; the command layer maps to
//! user-facing strings. Cascades are EXPLICIT in delete transactions because the app pool
//! connects without `PRAGMA foreign_keys = ON` (see `people.rs::delete`, `meeting.rs`):
//! - meeting delete → drops this meeting's participant rows (see `meeting.rs`).
//! - person delete ("forget this person") → drops their participant rows (see `people.rs`).
//!
//! `remove` here only ever tombstones the join row — never deletes it, and never touches a
//! `Person` or a speaker link. [`remove`](MeetingParticipantsRepository::remove) is a SOFT
//! delete: it stamps `removed_at` rather than `DELETE`-ing the row, so the calendar
//! seed-on-view (`seed_from_attendees`, which re-seeds on every read via
//! `api_get_meeting_participants`) can never resurrect someone the user explicitly removed
//! mid-recording. Every read/count query below filters `removed_at IS NULL`.
//! [`restore_or_add`](MeetingParticipantsRepository::restore_or_add) clears the tombstone
//! for the manual re-add / speaker-identified paths.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};

use crate::calendar::eventkit::Attendee;
use crate::database::repositories::owner_emails::{normalize_email, OwnerEmailsRepository};
use crate::database::repositories::people::PeopleRepository;
use crate::people::enroll::OWNER_PERSON_ID;

/// One rostered participant, joined to its `people` row. Serialized camelCase for the
/// frontend. `displayName`/`email`/`role` come from the joined person; `source` records how
/// the participant got onto the roster (`'calendar'` | `'manual'`).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingParticipant {
    pub person_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: Option<String>,
    /// How this participant got onto the roster: `'calendar'` (seeded) | `'manual'`.
    pub source: String,
    /// A self-contained base64 `data:` image URI for this participant's directory
    /// profile photo, when one was cached from the Google domain directory
    /// (specs/0038 WS3). Joined by normalized email against `attendee_photos`, so an
    /// email-less participant (or one whose org forbids the People directory read) is
    /// `None` and the UI degrades to initials. Serialized `photoDataUri`.
    pub photo_data_uri: Option<String>,
}

/// One row of the bounded meetings-list attendee preview (specs/0038 WS8.a).
///
/// `attendee_previews` emits up to a few of these PER MEETING, plus every row carries
/// its meeting's total NON-owner roster size (`total`) so the frontend can render a
/// "+N" overflow without a second query. `is_current_user` flags the device owner
/// (`person_id == OWNER_PERSON_ID`) — the data is owner-INCLUSIVE and the frontend
/// applies the display-only owner filter (WS8.b), mirroring the Day Agenda.
#[derive(Debug, Clone, FromRow)]
pub struct AttendeePreviewRow {
    pub meeting_id: String,
    pub display_name: String,
    pub email: Option<String>,
    /// 1 when this participant is the device owner (belt-and-suspenders — the roster
    /// is owner-excluded at seed time, but a manually-added "You" could still appear).
    pub is_current_user: i64,
    /// Total roster size for this meeting, owner-INCLUSIVE (matches the Day Agenda's
    /// `attendee_count` convention). The frontend subtracts any owner it can see in the
    /// preview before rendering the "+N" overflow (WS8.b, display-only).
    pub total: i64,
}

/// Cap on attendee rows returned per meeting by [`MeetingParticipantsRepository::attendee_previews`].
/// The row shows a few avatars + "+N"; anything beyond this is folded into the overflow count.
pub const ATTENDEE_PREVIEW_LIMIT: i64 = 4;

pub struct MeetingParticipantsRepository;

impl MeetingParticipantsRepository {
    /// Bounded per-meeting attendee preview for the All Meetings list (specs/0038 WS8.a):
    /// up to [`ATTENDEE_PREVIEW_LIMIT`] participants for EVERY meeting that has a roster,
    /// plus each meeting's total non-owner roster size — all in a SINGLE query (no N+1
    /// across the meeting set). Uses window functions to rank/limit per meeting.
    ///
    /// Non-owner participants sort first (so the few preview slots prefer the "other
    /// people"), then by display name (case-insensitive). The owner is still INCLUDED and
    /// flagged (`is_current_user`) so owner exclusion stays a display decision. `total` is
    /// the owner-inclusive roster size (the frontend subtracts a previewed owner).
    pub async fn attendee_previews(
        pool: &SqlitePool,
    ) -> Result<Vec<AttendeePreviewRow>, SqlxError> {
        let rows = sqlx::query_as::<_, AttendeePreviewRow>(
            r#"
            WITH ranked AS (
                SELECT
                    mp.meeting_id AS meeting_id,
                    p.display_name AS display_name,
                    p.email AS email,
                    CASE WHEN mp.person_id = ?1 THEN 1 ELSE 0 END AS is_current_user,
                    ROW_NUMBER() OVER (
                        PARTITION BY mp.meeting_id
                        ORDER BY (mp.person_id = ?1) ASC, p.display_name COLLATE NOCASE ASC
                    ) AS rn,
                    COUNT(*) OVER (PARTITION BY mp.meeting_id) AS total
                FROM meeting_participants mp
                JOIN people p ON p.id = mp.person_id WHERE mp.removed_at IS NULL
            )
            SELECT meeting_id, display_name, email, is_current_user, total
            FROM ranked
            WHERE rn <= ?2
            ORDER BY meeting_id, rn
            "#,
        )
        .bind(OWNER_PERSON_ID)
        .bind(ATTENDEE_PREVIEW_LIMIT)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// The roster for a meeting: JOIN `meeting_participants` → `people`, ordered by display
    /// name (case-insensitive). This is what the detail + recording UIs render.
    pub async fn list(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<MeetingParticipant>, SqlxError> {
        let rows = sqlx::query_as::<_, MeetingParticipant>(
            "SELECT mp.person_id       AS person_id,
                    p.display_name      AS display_name,
                    p.email             AS email,
                    p.role              AS role,
                    mp.source           AS source,
                    ap.photo_data_uri   AS photo_data_uri
             FROM meeting_participants mp
             JOIN people p ON p.id = mp.person_id
             LEFT JOIN attendee_photos ap ON ap.email = LOWER(TRIM(p.email))
             WHERE mp.meeting_id = ? AND mp.removed_at IS NULL
             ORDER BY p.display_name COLLATE NOCASE ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Add a person to a meeting's roster. `INSERT OR IGNORE` on the
    /// `(meeting_id, person_id)` PK → idempotent (re-adding is a no-op). Returns whether a
    /// new row was actually inserted.
    pub async fn add(
        pool: &SqlitePool,
        meeting_id: &str,
        person_id: &str,
        source: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "INSERT OR IGNORE INTO meeting_participants
                (meeting_id, person_id, source, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(meeting_id)
        .bind(person_id)
        .bind(source)
        .bind(&now)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Add for the MANUAL/IDENTIFIED paths: insert the row, or — when a
    /// tombstoned row exists — clear `removed_at` (the user or diarization is
    /// explicitly putting this person back). The calendar seed keeps using
    /// [`add`](Self::add), whose INSERT OR IGNORE respects tombstones.
    pub async fn restore_or_add(
        pool: &SqlitePool,
        meeting_id: &str,
        person_id: &str,
        source: &str,
    ) -> Result<bool, SqlxError> {
        if Self::add(pool, meeting_id, person_id, source).await? {
            return Ok(true);
        }
        let res = sqlx::query(
            "UPDATE meeting_participants SET removed_at = NULL, source = ?
             WHERE meeting_id = ? AND person_id = ? AND removed_at IS NOT NULL",
        )
        .bind(source)
        .bind(meeting_id)
        .bind(person_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Put an IDENTIFIED speaker's person on the roster (specs/0038 WS6.c). Called when the
    /// user names a transcript speaker (`api_assign_speaker_to_person` /
    /// `api_assign_speaker_to_attendee`), so a named speaker always shows as a participant.
    ///
    /// The device owner ("You") is implicit — the mic channel, never a roster row (specs/0018)
    /// — so the singleton [`OWNER_PERSON_ID`] is skipped (returns `Ok(false)`). Otherwise this
    /// is [`restore_or_add`](Self::restore_or_add) with source `'identified'`: a demonstrably
    /// attended speaker restores a tombstoned roster row rather than staying removed, and is
    /// idempotent via the `(meeting_id, person_id)` PK, so re-identifying the same speaker
    /// never duplicates the row. Returns whether a row was inserted or a tombstone cleared.
    pub async fn add_identified(
        pool: &SqlitePool,
        meeting_id: &str,
        person_id: &str,
    ) -> Result<bool, SqlxError> {
        if person_id.trim() == OWNER_PERSON_ID {
            return Ok(false);
        }
        Self::restore_or_add(pool, meeting_id, person_id, "identified").await
    }

    /// Remove a participant from a meeting's roster — a SOFT delete: stamps
    /// `removed_at` and keeps the row as a tombstone so the calendar
    /// seed-on-view (`seed_from_attendees`, INSERT OR IGNORE) can never
    /// resurrect the participant. The Person and any `speakers.person_id`
    /// link survive. Returns whether an active row was tombstoned.
    pub async fn remove(
        pool: &SqlitePool,
        meeting_id: &str,
        person_id: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE meeting_participants SET removed_at = ?
             WHERE meeting_id = ? AND person_id = ? AND removed_at IS NULL",
        )
        .bind(&now)
        .bind(meeting_id)
        .bind(person_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Number of participants on a meeting's roster (UI count / sizing helper). Excludes
    /// tombstoned (removed) rows.
    pub async fn count_for_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<i64, SqlxError> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM meeting_participants
             WHERE meeting_id = ? AND removed_at IS NULL",
        )
        .bind(meeting_id)
        .fetch_one(pool)
        .await?;
        Ok(row.0)
    }

    /// Person ids on the roster EXCLUDING the owner — used to size the diarization cap
    /// (specs/0017 Phase B). The roster is already owner-excluded at seed time, but we
    /// hard-exclude [`OWNER_PERSON_ID`] here too so a manually-added "You" can never inflate
    /// the speaker cap. Excludes tombstoned (removed) rows.
    pub async fn remote_person_ids(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<String>, SqlxError> {
        let rows = sqlx::query_as::<_, (String,)>(
            "SELECT person_id FROM meeting_participants
             WHERE meeting_id = ? AND person_id <> ? AND removed_at IS NULL",
        )
        .bind(meeting_id)
        .bind(OWNER_PERSON_ID)
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Seed the roster from a calendar event's attendees. For each NON-current-user
    /// attendee: upsert a `Person` (by email via [`PeopleRepository::create`]'s
    /// upsert-by-email when an email is present; create-by-name with NULL email otherwise,
    /// so email-less invitees still appear), then `add(.., 'calendar')`.
    ///
    /// The current user is SKIPPED — the roster is the other people ("You" is implicit, the
    /// mic channel). "Is this me?" is `a.is_current_user OR the attendee's email is an owner
    /// email` (specs/0018): EventKit's `isCurrentUser()` only flags the calendar account's
    /// own address, so a claimed alias is skipped here too and never re-seeded. Idempotent
    /// across re-resolution:
    /// - Email-bearing attendees dedupe via `PeopleRepository::create`'s upsert-by-email +
    ///   the `INSERT OR IGNORE` PK — which also means a tombstoned (removed) row is never
    ///   resurrected by a re-seed, since `INSERT OR IGNORE` no-ops against an existing PK
    ///   regardless of `removed_at`.
    /// - Email-LESS attendees have no stable key (each `create` would mint a fresh person),
    ///   so we dedupe them against the existing roster (including tombstoned rows) by display
    ///   name (case-insensitive): if a participant with that name is already rostered — active
    ///   or removed — we skip rather than create a duplicate name-only person. (Two genuinely
    ///   different "John"s across *different* meetings still get separate people — merge-people
    ///   is a later wave.)
    ///
    /// Returns the number of roster rows newly inserted on this call.
    pub async fn seed_from_attendees(
        pool: &SqlitePool,
        meeting_id: &str,
        attendees: &[Attendee],
    ) -> Result<usize, SqlxError> {
        // Snapshot existing names INCLUDING tombstoned rows so re-seeding an
        // email-less invitee the user removed is a no-op, not a resurrection.
        let existing_names: std::collections::HashSet<String> = sqlx::query_as::<_, (String,)>(
            "SELECT p.display_name FROM meeting_participants mp
             JOIN people p ON p.id = mp.person_id WHERE mp.meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(name,)| name.trim().to_lowercase())
        .collect();

        // Load the owner-email set once (specs/0018): an attendee whose address is one of
        // these IS the owner, so skip them exactly like `is_current_user`. Normalized set →
        // membership check without an N-query-per-attendee `contains`.
        let owner_emails: std::collections::HashSet<String> = OwnerEmailsRepository::list(pool)
            .await?
            .into_iter()
            .collect();

        let mut added = 0usize;
        // A distribution list is not a person: it's excluded from the roster (and
        // thus speaker/voiceprint binding) — the labeled-DL floor (specs/0038
        // WS3, 0027 Phase 1). Its real members are folded in as ordinary
        // attendees upstream (Cloud Identity expansion) and seed normally.
        for attendee in attendees
            .iter()
            .filter(|a| !a.is_current_user && !a.is_distribution_list)
        {
            let name = attendee.name.trim();
            let email = attendee
                .email
                .as_deref()
                .map(str::trim)
                .filter(|e| !e.is_empty());

            // An attendee invited under one of the owner's addresses is "me" — skip it (it
            // was claimed / declared), so future calendar seeds never re-add the owner.
            if let Some(e) = email {
                if owner_emails.contains(&normalize_email(e)) {
                    continue;
                }
            }
            if name.is_empty() && email.is_none() {
                // Nothing to anchor a person on — skip rather than create a blank row.
                continue;
            }

            // Email-less attendees can't dedupe by email; skip if already on the roster by
            // name so re-seeding doesn't mint a fresh name-only person every time.
            if email.is_none() && existing_names.contains(&name.to_lowercase()) {
                continue;
            }

            let person = PeopleRepository::create(pool, name, email, None, None).await?;

            if Self::add(pool, meeting_id, &person.id, "calendar").await? {
                added += 1;
            }
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool with just the schema these tests touch (people +
    /// meeting_participants). Mirrors the migration shape; no `PRAGMA foreign_keys`.
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");

        sqlx::query(
            "CREATE TABLE people (
                id TEXT PRIMARY KEY,
                email TEXT,
                display_name TEXT NOT NULL,
                role TEXT,
                notes TEXT,
                voiceprint_opt_out INTEGER NOT NULL DEFAULT 0,
                starred INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE UNIQUE INDEX idx_people_email ON people(email) WHERE email IS NOT NULL",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE meeting_participants (
                meeting_id TEXT NOT NULL,
                person_id TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'manual',
                created_at TEXT NOT NULL,
                removed_at TEXT,
                PRIMARY KEY (meeting_id, person_id)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE speakers (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                speaker_key TEXT NOT NULL,
                display_name TEXT,
                email TEXT,
                person_id TEXT,
                is_local INTEGER NOT NULL DEFAULT 0,
                created_at TEXT,
                updated_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE owner_emails (email TEXT PRIMARY KEY, created_at TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        // Mirrors the real `attendee_photos` migration so the roster's photo join
        // (keyed by normalized email) is exercised here (specs/0038 WS3, RC-4).
        sqlx::query(
            "CREATE TABLE attendee_photos (
                email TEXT PRIMARY KEY,
                photo_data_uri TEXT NOT NULL,
                fetched_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE voiceprints (
                id TEXT PRIMARY KEY, person_id TEXT NOT NULL, embedding BLOB NOT NULL,
                embedding_dim INTEGER NOT NULL, embedding_model TEXT NOT NULL,
                source_meeting_id TEXT, sample_quality REAL, created_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn attendee(name: &str, email: Option<&str>, is_me: bool) -> Attendee {
        Attendee {
            name: name.to_string(),
            email: email.map(str::to_string),
            is_current_user: is_me,
            is_distribution_list: false,
            photo_data_uri: None,
        }
    }

    /// A distribution-list attendee (group address, flagged) — never bound.
    fn distribution_list(email: &str) -> Attendee {
        Attendee {
            name: email.to_string(),
            email: Some(email.to_string()),
            is_current_user: false,
            is_distribution_list: true,
            photo_data_uri: None,
        }
    }

    #[tokio::test]
    async fn seed_is_idempotent_and_excludes_owner() {
        let pool = test_pool().await;
        let attendees = vec![
            attendee("Me", Some("me@example.com"), true),
            attendee("Alice", Some("alice@example.com"), false),
            attendee("Bob", None, false), // email-less invitee
        ];

        let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
            .await
            .unwrap();
        assert_eq!(added, 2, "owner excluded; Alice + Bob seeded");

        // Re-seed: upsert-by-email + INSERT OR IGNORE → no new rows, no new people.
        let added_again =
            MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
                .await
                .unwrap();
        assert_eq!(added_again, 0, "re-seed adds nothing");

        let roster = MeetingParticipantsRepository::list(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(roster.len(), 2);
        // Ordered by display_name: Alice, Bob.
        assert_eq!(roster[0].display_name, "Alice");
        assert_eq!(roster[0].email.as_deref(), Some("alice@example.com"));
        assert_eq!(roster[0].source, "calendar");
        assert_eq!(roster[1].display_name, "Bob");
        assert!(roster[1].email.is_none());

        let people: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM people")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(people.0, 2, "no duplicate people after re-seed");
    }

    #[tokio::test]
    async fn seed_excludes_distribution_list_but_keeps_people() {
        let pool = test_pool().await;
        let attendees = vec![
            attendee("Alice", Some("alice@example.com"), false),
            distribution_list("eng-team@example.com"), // group → not a person
        ];
        let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
            .await
            .unwrap();
        assert_eq!(
            added, 1,
            "the DL is excluded from binding; only Alice seeded"
        );

        let roster = MeetingParticipantsRepository::list(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].display_name, "Alice");
        // No person was minted for the group address.
        let people: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM people WHERE email = 'eng-team@example.com'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(people.0, 0, "no person minted for a distribution list");
    }

    #[tokio::test]
    async fn seed_skips_owner_email_attendee() {
        let pool = test_pool().await;
        // Declare an alias as an owner email (no is_current_user flag on the attendee).
        OwnerEmailsRepository::add(&pool, "Me@Work.com")
            .await
            .unwrap();

        let attendees = vec![
            attendee("Me (alias)", Some("me@work.com"), false), // owner email → skipped
            attendee("Alice", Some("alice@example.com"), false),
        ];
        let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
            .await
            .unwrap();
        assert_eq!(added, 1, "owner-email attendee skipped; only Alice seeded");

        let roster = MeetingParticipantsRepository::list(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].display_name, "Alice");
    }

    #[tokio::test]
    async fn remove_leaves_person_intact() {
        let pool = test_pool().await;
        let person = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, "m1", &person.id, "manual")
            .await
            .unwrap();

        let removed = MeetingParticipantsRepository::remove(&pool, "m1", &person.id)
            .await
            .unwrap();
        assert!(removed);

        assert!(MeetingParticipantsRepository::list(&pool, "m1")
            .await
            .unwrap()
            .is_empty());
        // The Person survives.
        assert!(PeopleRepository::get(&pool, &person.id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn remote_person_ids_excludes_owner() {
        let pool = test_pool().await;
        // Owner singleton + a real remote person, both rostered.
        sqlx::query(
            "INSERT INTO people (id, display_name, voiceprint_opt_out, created_at, updated_at)
             VALUES (?, 'You', 0, '', '')",
        )
        .bind(OWNER_PERSON_ID)
        .execute(&pool)
        .await
        .unwrap();
        let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
            .await
            .unwrap();

        MeetingParticipantsRepository::add(&pool, "m1", OWNER_PERSON_ID, "manual")
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
            .await
            .unwrap();

        let remote = MeetingParticipantsRepository::remote_person_ids(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(remote, vec![alice.id]);
    }

    /// specs/0038 WS6.c: identifying a non-owner speaker adds exactly one roster row with
    /// source `'identified'`; re-identifying the same person is a no-op; identifying the
    /// owner ("You") adds nothing (they're implicit — specs/0018).
    #[tokio::test]
    async fn add_identified_dedupes_and_skips_owner() {
        let pool = test_pool().await;
        // Owner singleton + a real remote person.
        sqlx::query(
            "INSERT INTO people (id, display_name, voiceprint_opt_out, created_at, updated_at)
             VALUES (?, 'You', 0, '', '')",
        )
        .bind(OWNER_PERSON_ID)
        .execute(&pool)
        .await
        .unwrap();
        let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
            .await
            .unwrap();

        // Non-owner: first identify inserts one row; re-identify is a no-op.
        assert!(
            MeetingParticipantsRepository::add_identified(&pool, "m1", &alice.id)
                .await
                .unwrap(),
            "identifying a non-owner speaker adds a roster row"
        );
        assert!(
            !MeetingParticipantsRepository::add_identified(&pool, "m1", &alice.id)
                .await
                .unwrap(),
            "re-identifying the same speaker does not duplicate"
        );

        // Owner is implicit → never rostered, even via add_identified.
        assert!(
            !MeetingParticipantsRepository::add_identified(&pool, "m1", OWNER_PERSON_ID)
                .await
                .unwrap(),
            "identifying the owner adds nothing"
        );

        let roster = MeetingParticipantsRepository::list(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(
            roster.len(),
            1,
            "exactly one roster row (Alice, not the owner)"
        );
        assert_eq!(roster[0].person_id, alice.id);
        assert_eq!(roster[0].source, "identified");
    }

    #[tokio::test]
    async fn add_is_idempotent() {
        let pool = test_pool().await;
        let person = PeopleRepository::create(&pool, "Alice", None, None, None)
            .await
            .unwrap();
        assert!(
            MeetingParticipantsRepository::add(&pool, "m1", &person.id, "manual")
                .await
                .unwrap()
        );
        assert!(
            !MeetingParticipantsRepository::add(&pool, "m1", &person.id, "manual")
                .await
                .unwrap(),
            "second add is a no-op"
        );
        assert_eq!(
            MeetingParticipantsRepository::count_for_meeting(&pool, "m1")
                .await
                .unwrap(),
            1
        );
    }
}
