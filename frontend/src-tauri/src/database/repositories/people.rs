//! `people` table access — the durable, app-wide identity entity (specs/0016 Phase 1b).
//!
//! A `people` row is the cross-meeting anchor for one human. The same person across ten
//! meetings is one `people` row (not ten unrelated `speakers` rows sharing an email),
//! and `speakers.person_id` links a per-meeting speaker occurrence back to it.
//!
//! Identity is **decoupled from voiceprints** (ADR-0007 §2): a person can exist with no
//! email and no stored voice, and the `voiceprint_opt_out` flag (set here in 1b; wired to
//! actual voiceprint deletion in 1c) lets a known person be associated with detected
//! speakers while never having their voice modeled. Assigning a speaker to a person works
//! regardless of that flag — it's identity association, not voice storage.
//!
//! Mirrors `speaker.rs`/`meeting.rs`: returns `SqlxError`; the command layer maps to
//! user-facing strings. Cascade cleanups are EXPLICIT in transactions because the app
//! pool connects without `PRAGMA foreign_keys = ON` (see `meeting.rs` delete path).

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

/// A durable, app-wide person (specs/0016 1b). Serialized camelCase for the frontend.
///
/// `email` is the stable cross-meeting key but is OPTIONAL — a voice-only or
/// manually-created person may have none. `voiceprint_opt_out` (ADR-0007 §2) is stored
/// as a SQLite integer (0/1) but exposed to the frontend as a bool. `photo_data_uri`
/// (specs/0056 W6) is the cached Google directory photo, read-through from
/// `attendee_photos` by the `list` / `list_ranked` / `get` joins — never stored here.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub role: Option<String>,
    pub notes: Option<String>,
    /// ADR-0007 §2: when true, Nixon never builds/stores a voiceprint for this person.
    #[sqlx(rename = "voiceprint_opt_out")]
    #[serde(rename = "voiceprintOptOut")]
    pub voiceprint_opt_out: bool,
    /// specs/0038 WS5.a: pinned to the top of ranked pick-lists. Stored as a SQLite
    /// integer (0/1), exposed to the frontend as a bool.
    pub starred: bool,
    pub created_at: String,
    pub updated_at: String,
    /// specs/0056 W6: the cached same-org directory photo (`attendee_photos`, a
    /// self-contained `data:` URI) for this person's email, when one exists. Populated
    /// only by the reads that LEFT JOIN the cache; `#[sqlx(default)]` keeps the join-free
    /// queries (`get_by_email`) decoding. Rendering falls back to initials when `None`.
    #[sqlx(default)]
    pub photo_data_uri: Option<String>,
}

pub struct PeopleRepository;

impl PeopleRepository {
    /// Create a person, **upsert-by-email**: if `email` is `Some` and a person with that
    /// (case-insensitively matched at the call boundary — stored verbatim) email already
    /// exists, the existing person is returned unchanged rather than violating the
    /// partial unique index. This is the documented collision policy (spec "NULL-email
    /// collisions"): callers always get a usable person back and never an index error.
    /// An email-less create always inserts a fresh row.
    ///
    /// Generates `person-<uuid>` and stamps `created_at`/`updated_at`.
    pub async fn create(
        pool: &SqlitePool,
        display_name: &str,
        email: Option<&str>,
        role: Option<&str>,
        notes: Option<&str>,
    ) -> Result<Person, SqlxError> {
        // Normalize: treat empty/whitespace email as None so we never store "" (which
        // would collide under the partial unique index).
        let email = email.map(str::trim).filter(|e| !e.is_empty());

        // Upsert-by-email: return the existing person on collision instead of erroring.
        if let Some(e) = email {
            if let Some(existing) = Self::get_by_email(pool, e).await? {
                return Ok(existing);
            }
        }

        let id = format!("person-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT INTO people
                (id, email, display_name, role, notes, voiceprint_opt_out, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(email)
        .bind(display_name)
        .bind(role)
        .bind(notes)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Ok(Person {
            id,
            email: email.map(str::to_string),
            display_name: display_name.to_string(),
            role: role.map(str::to_string),
            notes: notes.map(str::to_string),
            voiceprint_opt_out: false,
            starred: false,
            created_at: now.clone(),
            updated_at: now,
            photo_data_uri: None,
        })
    }

    /// All people, ordered by display name (the People directory order), each carrying
    /// their cached directory photo when one exists (specs/0056 W6 — the join key is the
    /// normalized email, matching how `attendee_photos` rows are written).
    pub async fn list(pool: &SqlitePool) -> Result<Vec<Person>, SqlxError> {
        let rows = sqlx::query_as::<_, Person>(
            "SELECT p.id, p.email, p.display_name, p.role, p.notes, p.voiceprint_opt_out,
                    p.starred, p.created_at, p.updated_at,
                    ap.photo_data_uri AS photo_data_uri
             FROM people p
             LEFT JOIN attendee_photos ap ON ap.email = LOWER(TRIM(p.email))
             ORDER BY p.display_name COLLATE NOCASE ASC",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// All people, ranked for pick-lists (specs/0038 WS5.a): **starred first, then by call
    /// frequency (most-seen first), then alphabetically** — so a user's top collaborators
    /// float to the top instead of an unranked alphabetical wall.
    ///
    /// Frequency is the number of distinct meetings a person appears in, counted live
    /// (small N) across BOTH the roster (`meeting_participants`, excluding tombstoned/removed
    /// rows) and detected transcript speakers (`speakers.person_id`) — a person can be
    /// rostered, spoke, or both. We UNION the two `(person_id, meeting_id)` sources and
    /// DISTINCT them so a meeting where the person was both rostered AND spoke counts once,
    /// and correlated subqueries over that union give each person their meeting count without
    /// a fan-out JOIN.
    pub async fn list_ranked(pool: &SqlitePool) -> Result<Vec<Person>, SqlxError> {
        let rows = sqlx::query_as::<_, Person>(
            "SELECT p.id, p.email, p.display_name, p.role, p.notes, p.voiceprint_opt_out,
                    p.starred, p.created_at, p.updated_at,
                    ap.photo_data_uri AS photo_data_uri,
                    (SELECT COUNT(*) FROM (
                        SELECT meeting_id FROM meeting_participants
                            WHERE person_id = p.id AND removed_at IS NULL
                        UNION
                        SELECT meeting_id FROM speakers WHERE person_id = p.id
                    )) AS call_count
             FROM people p
             LEFT JOIN attendee_photos ap ON ap.email = LOWER(TRIM(p.email))
             ORDER BY p.starred DESC,
                      call_count DESC,
                      p.display_name COLLATE NOCASE ASC",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    /// Pin/unpin a person to the top of ranked pick-lists (specs/0038 WS5.a) and bump
    /// `updated_at`. Returns `Ok(false)` when no matching person exists.
    pub async fn set_starred(
        pool: &SqlitePool,
        id: &str,
        starred: bool,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query("UPDATE people SET starred = ?, updated_at = ? WHERE id = ?")
            .bind(starred as i64)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// One person by id (with their cached directory photo, specs/0056 W6), or `None`.
    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Person>, SqlxError> {
        let row = sqlx::query_as::<_, Person>(
            "SELECT p.id, p.email, p.display_name, p.role, p.notes, p.voiceprint_opt_out,
                    p.starred, p.created_at, p.updated_at,
                    ap.photo_data_uri AS photo_data_uri
             FROM people p
             LEFT JOIN attendee_photos ap ON ap.email = LOWER(TRIM(p.email))
             WHERE p.id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }

    /// One person by email (case-insensitive), or `None`. Used for upsert-by-email and by
    /// the matcher's resolution of an `email` candidate to its durable person.
    pub async fn get_by_email(pool: &SqlitePool, email: &str) -> Result<Option<Person>, SqlxError> {
        let row = sqlx::query_as::<_, Person>(
            "SELECT id, email, display_name, role, notes, voiceprint_opt_out, starred,
                    created_at, updated_at
             FROM people WHERE email = ? COLLATE NOCASE",
        )
        .bind(email.trim())
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }

    /// Update a person's editable fields and bump `updated_at`. Returns `Ok(false)` when
    /// no matching person exists.
    ///
    /// Setting an `email` that collides with a DIFFERENT existing person is rejected with
    /// a clear error (the partial unique index would also reject it, but we check first to
    /// return an actionable message). An empty/whitespace email is normalized to NULL.
    pub async fn update(
        pool: &SqlitePool,
        id: &str,
        display_name: &str,
        email: Option<&str>,
        role: Option<&str>,
        notes: Option<&str>,
    ) -> Result<bool, SqlxError> {
        let email = email.map(str::trim).filter(|e| !e.is_empty());

        // Guard the email collision so callers get an actionable message instead of a raw
        // UNIQUE-constraint failure (spec "NULL-email collisions": link the existing one).
        if let Some(e) = email {
            if let Some(existing) = Self::get_by_email(pool, e).await? {
                if existing.id != id {
                    return Err(SqlxError::Protocol(format!(
                        "email '{e}' already belongs to another person ({})",
                        existing.display_name
                    )));
                }
            }
        }

        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE people
             SET display_name = ?, email = ?, role = ?, notes = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(display_name)
        .bind(email)
        .bind(role)
        .bind(notes)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Set the per-person `voiceprint_opt_out` flag (ADR-0007 §2) and bump `updated_at`.
    /// Returns `Ok(false)` when no matching person exists.
    ///
    /// Turning the flag **on** also DELETES every stored voiceprint for this person in the
    /// SAME transaction (ADR-0007 §6) — the pool has no `PRAGMA foreign_keys`, so this
    /// explicit delete is the cascade, and doing it atomically with the flag flip means a
    /// crash can never leave "opted out but voice still stored". Future enrollment is then
    /// blocked by the gate at the enroll site (`global_opt_in && !voiceprint_opt_out`).
    /// Flipping the flag back to **false** does NOT recreate any samples — they're gone.
    pub async fn set_voiceprint_opt_out(
        pool: &SqlitePool,
        id: &str,
        opt_out: bool,
    ) -> Result<bool, SqlxError> {
        use crate::database::repositories::voiceprints::VoiceprintsRepository;

        let now = Utc::now().to_rfc3339();
        let mut tx = pool.begin().await?;

        let res =
            sqlx::query("UPDATE people SET voiceprint_opt_out = ?, updated_at = ? WHERE id = ?")
                .bind(opt_out as i64)
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await?;

        // Cascade-delete this person's voiceprints when turning opt-out ON (ADR-0007 §6).
        if opt_out && res.rows_affected() > 0 {
            VoiceprintsRepository::delete_for_person_tx(&mut tx, id).await?;
        }

        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    /// "Forget this person": delete the `people` row and detach it from every speaker.
    ///
    /// Runs in one transaction (mirrors `meeting.rs` delete): first NULL out
    /// `speakers.person_id` for this person (the pool has no `PRAGMA foreign_keys`, so the
    /// declared cascade does NOT fire — we must do it by hand or strand the link), then
    /// delete the row. Returns `Ok(false)` when no matching person exists.
    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        use crate::database::repositories::voiceprints::VoiceprintsRepository;

        let mut tx = pool.begin().await?;

        // Detach speakers from this person (explicit, since cascades don't fire).
        sqlx::query("UPDATE speakers SET person_id = NULL WHERE person_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        // Un-roster this person from every meeting (specs/0017). Explicit cascade (no
        // `PRAGMA foreign_keys`): forgetting a person also drops their participant rows.
        // The join rows only — other people's rosters and any speaker links are untouched.
        sqlx::query("DELETE FROM meeting_participants WHERE person_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        // Cascade-delete this person's voiceprints (ADR-0007 §6 "forget this person").
        // Explicit because the pool has no `PRAGMA foreign_keys` — a missed delete would
        // strand biometric data (a privacy bug, not just an orphan row).
        VoiceprintsRepository::delete_for_person_tx(&mut tx, id).await?;

        // Un-assign this person's action items (specs/0034): the item OUTLIVES the person
        // — NULL the FK, but first backfill `assignee_raw` with the person's display name
        // (when raw isn't already set) so the UI keeps showing WHO the task belonged to
        // instead of silently flipping to "unassigned". Explicit cascade, same reasoning
        // as above; the name is read inside the same transaction as the delete.
        let display_name: Option<String> =
            sqlx::query_scalar("SELECT display_name FROM people WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        sqlx::query(
            "UPDATE action_items
             SET assignee_raw = COALESCE(assignee_raw, ?), assignee_person_id = NULL
             WHERE assignee_person_id = ?",
        )
        .bind(display_name.as_deref())
        .bind(id)
        .execute(&mut *tx)
        .await?;

        let res = sqlx::query("DELETE FROM people WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    /// Associate a detected speaker with a durable person (specs/0016 1b): set
    /// `speakers.person_id` AND copy the person's `display_name`/`email` onto that
    /// speaker row, so the existing transcript display (resolved via the speakers join)
    /// and the `get_identified_with_embeddings` candidate query keep working unchanged.
    ///
    /// Works regardless of `voiceprint_opt_out` — this is identity association, not voice
    /// storage (ADR-0007 §2). Returns `Ok(false)` when either the person or the speaker
    /// row doesn't exist (nothing was linked).
    pub async fn assign_speaker_to_person(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker_key: &str,
        person_id: &str,
    ) -> Result<bool, SqlxError> {
        let person = match Self::get(pool, person_id).await? {
            Some(p) => p,
            None => return Ok(false),
        };

        let now = Utc::now();
        let res = sqlx::query(
            "UPDATE speakers
             SET person_id = ?, display_name = ?, email = COALESCE(?, email), updated_at = ?
             WHERE meeting_id = ? AND speaker_key = ?",
        )
        .bind(&person.id)
        .bind(&person.display_name)
        .bind(person.email.as_deref())
        .bind(now)
        .bind(meeting_id)
        .bind(speaker_key)
        .execute(pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    /// Role map for the role-weighted summary (specs/0012 Task 5).
    ///
    /// Joins this meeting's `speakers` → `people` ON `speakers.person_id = people.id`
    /// and returns `(speaker_display_name, role)` for every speaker whose linked person
    /// has a non-empty `role`. Keying on the **speaker's** `display_name` keeps the map
    /// aligned with the `Name:` prefixes that
    /// [`crate::summary::processor::build_speaker_attributed_transcript`] writes into the
    /// attributed transcript (those prefixes come from `speakers.display_name`).
    ///
    /// Speakers with no `person_id`, or whose person has a NULL/blank role, are omitted —
    /// so a meeting with no roles set returns an empty vec and the summary takes the
    /// byte-identical no-role path (an acceptance criterion of specs/0012).
    pub async fn get_meeting_speaker_roles(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<(String, String)>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT s.display_name, p.role
             FROM speakers s
             JOIN people p ON p.id = s.person_id
             WHERE s.meeting_id = ?
               AND s.display_name IS NOT NULL AND TRIM(s.display_name) <> ''
               AND p.role IS NOT NULL AND TRIM(p.role) <> ''
             ORDER BY s.display_name COLLATE NOCASE ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::attendee_photos::AttendeePhotosRepository;
    use crate::database::repositories::meeting::MeetingsRepository;
    use crate::database::repositories::meeting_participant::MeetingParticipantsRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool through the app's REAL migration set (incl. this WS's `starred`
    /// column). One connection — each in-memory connection is a separate database.
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// `list_ranked` orders starred-first, then by meeting frequency, then alphabetically —
    /// and the UNION over `meeting_participants` + `speakers` counts a meeting once even
    /// when a person is BOTH rostered and a detected speaker in it.
    #[tokio::test]
    async fn list_ranked_orders_starred_then_frequency() {
        let pool = test_pool().await;

        // Three meetings to anchor roster/speaker rows (speakers FK → meetings).
        let mut meetings = Vec::new();
        for i in 0..3 {
            meetings.push(
                MeetingsRepository::create_meeting(
                    &pool,
                    Some(format!("M{i}")),
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .expect("create meeting"),
            );
        }

        // Alice: starred, zero meetings — must still float to the very top.
        let alice = PeopleRepository::create(&pool, "Alice", None, None, None)
            .await
            .unwrap();
        PeopleRepository::set_starred(&pool, &alice.id, true)
            .await
            .unwrap();

        // Bob: unstarred, in TWO meetings (rostered in both; also a detected speaker in the
        // first — the UNION must dedupe that to a count of 2, not 3).
        let bob = PeopleRepository::create(&pool, "Bob", None, None, None)
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, &meetings[0], &bob.id, "manual")
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, &meetings[1], &bob.id, "manual")
            .await
            .unwrap();
        insert_speaker(&pool, &meetings[0], "spk_bob", &bob.id).await;

        // Charlie: unstarred, in ONE meeting (via the speakers path only).
        let charlie = PeopleRepository::create(&pool, "Charlie", None, None, None)
            .await
            .unwrap();
        insert_speaker(&pool, &meetings[2], "spk_charlie", &charlie.id).await;

        let ranked = PeopleRepository::list_ranked(&pool).await.unwrap();
        let order: Vec<&str> = ranked.iter().map(|p| p.display_name.as_str()).collect();
        assert_eq!(
            order,
            vec!["Alice", "Bob", "Charlie"],
            "starred first, then frequency (Bob=2 via dedup > Charlie=1)"
        );
        assert!(ranked[0].starred, "Alice round-trips as starred");
        assert!(!ranked[1].starred);

        // The plain alphabetical path is unchanged: Alice, Bob, Charlie by name.
        let plain = PeopleRepository::list(&pool).await.unwrap();
        let plain_order: Vec<&str> = plain.iter().map(|p| p.display_name.as_str()).collect();
        assert_eq!(plain_order, vec!["Alice", "Bob", "Charlie"]);
    }

    /// A tombstoned (removed) roster row must not count toward `call_count` — otherwise a
    /// removed attendee would keep outranking someone genuinely on the roster.
    #[tokio::test]
    async fn list_ranked_excludes_tombstoned_roster_rows() {
        let pool = test_pool().await;
        let meeting =
            MeetingsRepository::create_meeting(&pool, Some("M0".into()), None, None, None, None)
                .await
                .expect("create meeting");

        // Dave was rostered then removed — his only meeting must not count.
        let dave = PeopleRepository::create(&pool, "Dave", None, None, None)
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, &meeting, &dave.id, "manual")
            .await
            .unwrap();
        MeetingParticipantsRepository::remove(&pool, &meeting, &dave.id)
            .await
            .unwrap();

        // Eve is actively rostered in the same meeting.
        let eve = PeopleRepository::create(&pool, "Eve", None, None, None)
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, &meeting, &eve.id, "manual")
            .await
            .unwrap();

        let ranked = PeopleRepository::list_ranked(&pool).await.unwrap();
        let order: Vec<&str> = ranked.iter().map(|p| p.display_name.as_str()).collect();
        assert_eq!(
            order,
            vec!["Eve", "Dave"],
            "Eve's active roster row outranks Dave's tombstoned one, despite alphabetical order"
        );
    }

    /// specs/0056 W6: `get` / `list` / `list_ranked` carry the cached directory photo. The
    /// join key is the NORMALIZED email — `attendee_photos.email` is lowercased + trimmed at
    /// write time while `people.email` is stored verbatim — so a person whose email differs
    /// only in case still gets their photo. No cache row ⇒ `None`.
    #[tokio::test]
    async fn people_reads_carry_the_cached_attendee_photo() {
        let pool = test_pool().await;
        let alice =
            PeopleRepository::create(&pool, "Alice", Some(" Alice@Example.COM "), None, None)
                .await
                .unwrap();
        assert_eq!(alice.email.as_deref(), Some("Alice@Example.COM"));
        assert!(
            alice.photo_data_uri.is_none(),
            "create has no join → no photo"
        );
        let bob = PeopleRepository::create(&pool, "Bob", Some("bob@example.com"), None, None)
            .await
            .unwrap();

        let uri = "data:image/png;base64,QUJD";
        AttendeePhotosRepository::upsert_photo(
            &pool,
            "alice@example.com",
            uri,
            "2026-09-01T00:00:00Z",
        )
        .await
        .unwrap();

        let got = PeopleRepository::get(&pool, &alice.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            got.photo_data_uri.as_deref(),
            Some(uri),
            "get joins on LOWER(TRIM(email))"
        );
        let bob_got = PeopleRepository::get(&pool, &bob.id)
            .await
            .unwrap()
            .unwrap();
        assert!(bob_got.photo_data_uri.is_none(), "no cache row → None");

        for (label, people) in [
            ("list", PeopleRepository::list(&pool).await.unwrap()),
            (
                "list_ranked",
                PeopleRepository::list_ranked(&pool).await.unwrap(),
            ),
        ] {
            let photo_of = |name: &str| {
                people
                    .iter()
                    .find(|p| p.display_name == name)
                    .unwrap_or_else(|| panic!("{label}: {name} missing"))
                    .photo_data_uri
                    .clone()
            };
            assert_eq!(
                photo_of("Alice").as_deref(),
                Some(uri),
                "{label} carries Alice's photo"
            );
            assert_eq!(photo_of("Bob"), None, "{label} leaves Bob photo-less");
        }

        // The join-free lookup still deserializes (the column defaults to None).
        let by_email = PeopleRepository::get_by_email(&pool, "alice@example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_email.id, alice.id);
        assert!(by_email.photo_data_uri.is_none());
    }

    async fn insert_speaker(pool: &SqlitePool, meeting_id: &str, key: &str, person_id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO speakers
                (id, meeting_id, speaker_key, display_name, person_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("speaker-{}", Uuid::new_v4()))
        .bind(meeting_id)
        .bind(key)
        .bind("Speaker")
        .bind(person_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("insert speaker");
    }
}
