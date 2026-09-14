//! Tombstone semantics for meeting_participants.removed_at:
//! a mid-recording removal must survive the calendar seed-on-view.

use app_lib::calendar::eventkit::Attendee;
use app_lib::database::repositories::attendee_photos::AttendeePhotosRepository;
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::meeting_participant::{
    AttendeePreviewRow, MeetingParticipantsRepository, ATTENDEE_PREVIEW_LIMIT,
};
use app_lib::database::repositories::people::PeopleRepository;
use app_lib::people::enroll::OWNER_PERSON_ID;
use sqlx::SqlitePool;

async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

fn attendee(name: &str, email: Option<&str>) -> Attendee {
    Attendee {
        name: name.to_string(),
        email: email.map(str::to_string),
        is_current_user: false,
        is_distribution_list: false,
        photo_data_uri: None,
    }
}

#[tokio::test]
async fn seed_does_not_resurrect_removed_calendar_attendee() {
    let pool = pool_with_schema().await;
    let attendees = vec![attendee("Alice", Some("alice@example.com"))];

    MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    let roster = MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap();
    assert_eq!(roster.len(), 1);
    let alice_id = roster[0].person_id.clone();

    assert!(
        MeetingParticipantsRepository::remove(&pool, "m1", &alice_id)
            .await
            .unwrap()
    );
    assert!(MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap()
        .is_empty());

    // The bug: this re-seed used to resurrect Alice.
    let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    assert_eq!(added, 0, "seed must respect the tombstone");
    assert!(MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn seed_does_not_resurrect_removed_emailless_attendee() {
    let pool = pool_with_schema().await;
    let attendees = vec![attendee("Bob", None)]; // no email → dedup is by name snapshot

    MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    let bob_id = MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap()[0]
        .person_id
        .clone();
    MeetingParticipantsRepository::remove(&pool, "m1", &bob_id)
        .await
        .unwrap();

    let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    assert_eq!(added, 0, "name-dedup snapshot must include tombstoned rows");
    let people: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM people WHERE display_name = 'Bob'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(people.0, 1, "no duplicate name-only person minted");
}

#[tokio::test]
async fn restore_or_add_clears_tombstone_and_counts_exclude_removed() {
    let pool = pool_with_schema().await;
    let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
        .await
        .unwrap();
    let bob = PeopleRepository::create(&pool, "Bob", Some("b@x.com"), None, None)
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &bob.id, "calendar")
        .await
        .unwrap();

    MeetingParticipantsRepository::remove(&pool, "m1", &alice.id)
        .await
        .unwrap();
    // The diarization speaker cap consumers must not see removed rows.
    assert_eq!(
        MeetingParticipantsRepository::count_for_meeting(&pool, "m1")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        MeetingParticipantsRepository::remote_person_ids(&pool, "m1")
            .await
            .unwrap(),
        vec![bob.id.clone()]
    );

    // Manual re-add restores the same row (clears removed_at) rather than no-opping.
    assert!(
        MeetingParticipantsRepository::restore_or_add(&pool, "m1", &alice.id, "manual")
            .await
            .unwrap()
    );
    assert_eq!(
        MeetingParticipantsRepository::count_for_meeting(&pool, "m1")
            .await
            .unwrap(),
        2
    );
    // add() (the seed path) on an ACTIVE row is still a no-op.
    assert!(
        !MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn add_identified_restores_removed_participant() {
    let pool = pool_with_schema().await;
    let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
        .await
        .unwrap();
    MeetingParticipantsRepository::remove(&pool, "m1", &alice.id)
        .await
        .unwrap();

    // If diarization identifies her as a speaker, she demonstrably attended → restore.
    assert!(
        MeetingParticipantsRepository::add_identified(&pool, "m1", &alice.id)
            .await
            .unwrap()
    );
    let roster = MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].source, "identified");
}

/// A tombstoned (removed) roster row must not resurface in
/// `recent_meetings_with_person` (People directory "Recent with {person}",
/// specs/0038 WS5.b) — one of several direct `meeting_participants` readers outside
/// the repository itself that must honor the removal (Task 2 extension).
#[tokio::test]
async fn recent_meetings_with_person_excludes_tombstoned_row() {
    let pool = pool_with_schema().await;
    let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
        .await
        .unwrap();
    let meeting =
        MeetingsRepository::create_meeting(&pool, Some("M1".into()), None, None, None, None)
            .await
            .unwrap();
    MeetingParticipantsRepository::add(&pool, &meeting, &alice.id, "manual")
        .await
        .unwrap();

    let recent = MeetingsRepository::recent_meetings_with_person(&pool, &alice.id, 10)
        .await
        .unwrap();
    assert_eq!(recent.len(), 1, "active roster row is picked up");

    MeetingParticipantsRepository::remove(&pool, &meeting, &alice.id)
        .await
        .unwrap();

    let recent = MeetingsRepository::recent_meetings_with_person(&pool, &alice.id, 10)
        .await
        .unwrap();
    assert!(
        recent.is_empty(),
        "removed participant no longer counts as recent with this person (no roster row, no speaker row)"
    );
}

// The two tests below were moved out of the inline `#[cfg(test)] mod tests` in
// `src/database/repositories/meeting_participant.rs` to keep that file under the
// 800-line ratchet (specs/0042) once the tombstone changes landed. Bodies are
// unchanged apart from swapping the inline `test_pool()` helper (a hand-rolled
// minimal schema) for this file's `pool_with_schema()` (the real migration set).

/// specs/0038 WS8.a: the meetings-list attendee preview is bounded per meeting,
/// carries the owner-EXCLUDED total (for "+N"), and is owner-INCLUSIVE + flagged
/// so the frontend owns the display filter.
#[tokio::test]
async fn attendee_previews_are_bounded_and_owner_inclusive() {
    let pool = pool_with_schema().await;
    // Owner singleton.
    sqlx::query(
        "INSERT INTO people (id, display_name, voiceprint_opt_out, created_at, updated_at)
         VALUES (?, 'You', 0, '', '')",
    )
    .bind(OWNER_PERSON_ID)
    .execute(&pool)
    .await
    .unwrap();

    // m1: owner + two remote → all three fit under the limit; owner flagged, sorted last.
    let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
        .await
        .unwrap();
    let bob = PeopleRepository::create(&pool, "Bob", Some("b@x.com"), None, None)
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", OWNER_PERSON_ID, "manual")
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &bob.id, "calendar")
        .await
        .unwrap();

    // m2: six remote participants → preview is capped at the limit, total = 6.
    for i in 0..6 {
        let p = PeopleRepository::create(&pool, &format!("P{i:02}"), None, None, None)
            .await
            .unwrap();
        MeetingParticipantsRepository::add(&pool, "m2", &p.id, "calendar")
            .await
            .unwrap();
    }

    let rows = MeetingParticipantsRepository::attendee_previews(&pool)
        .await
        .unwrap();

    let m1: Vec<&AttendeePreviewRow> = rows.iter().filter(|r| r.meeting_id == "m1").collect();
    assert_eq!(
        m1.len(),
        3,
        "owner + 2 remote all fit under the preview cap"
    );
    assert!(
        m1.iter().all(|r| r.total == 3),
        "total is owner-inclusive (owner + Alice + Bob)"
    );
    // Non-owner rows sort first (Alice, Bob), owner last.
    assert_eq!(m1[0].display_name, "Alice");
    assert_eq!(m1[0].is_current_user, 0);
    assert_eq!(m1[1].display_name, "Bob");
    assert_eq!(m1[2].display_name, "You");
    assert_eq!(m1[2].is_current_user, 1, "owner is included and flagged");

    let m2: Vec<&AttendeePreviewRow> = rows.iter().filter(|r| r.meeting_id == "m2").collect();
    assert_eq!(
        m2.len() as i64,
        ATTENDEE_PREVIEW_LIMIT,
        "preview is capped per meeting"
    );
    assert!(m2.iter().all(|r| r.total == 6), "total is the full roster");
}

/// specs/0038 WS3 (RC-4): the roster folds a cached directory photo onto a
/// participant by NORMALIZED (trim + case-insensitive) email, and leaves an
/// unmatched / email-less participant `None` (→ initials in the UI).
#[tokio::test]
async fn list_joins_cached_photo_by_normalized_email() {
    let pool = pool_with_schema().await;
    // Alice has a cached photo keyed by a lowercased email; the person row
    // stores a mixed-case, whitespace-padded address to prove the join
    // normalizes both sides.
    let alice = PeopleRepository::create(&pool, "Alice", Some("  Alice@X.com "), None, None)
        .await
        .unwrap();
    // Bob has no cached photo; Carol has no email at all.
    let bob = PeopleRepository::create(&pool, "Bob", Some("bob@x.com"), None, None)
        .await
        .unwrap();
    let carol = PeopleRepository::create(&pool, "Carol", None, None, None)
        .await
        .unwrap();
    AttendeePhotosRepository::upsert_photo(
        &pool,
        "alice@x.com",
        "data:image/jpeg;base64,aliceface",
        "2026-07-07T00:00:00+00:00",
    )
    .await
    .unwrap();

    for id in [&alice.id, &bob.id, &carol.id] {
        MeetingParticipantsRepository::add(&pool, "m1", id, "calendar")
            .await
            .unwrap();
    }

    let roster = MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap();
    let by_name = |name: &str| {
        roster
            .iter()
            .find(|p| p.display_name == name)
            .unwrap_or_else(|| panic!("{name} on roster"))
    };
    assert_eq!(
        by_name("Alice").photo_data_uri.as_deref(),
        Some("data:image/jpeg;base64,aliceface"),
        "the cached photo joins case-insensitively despite stored whitespace/case"
    );
    assert!(
        by_name("Bob").photo_data_uri.is_none(),
        "no cached photo ⇒ None (initials)"
    );
    assert!(
        by_name("Carol").photo_data_uri.is_none(),
        "an email-less participant ⇒ None (initials)"
    );
}
