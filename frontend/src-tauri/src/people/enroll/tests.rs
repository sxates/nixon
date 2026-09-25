//! Tests for `people::enroll` (moved out of the module to keep it under the file-size cap).

use super::*;
use crate::database::repositories::people::PeopleRepository;
use sqlx::SqlitePool;

// ----- pure consent-gate matrix (ADR-0007 §2/§3 as amended by specs/0078) -----

/// specs/0078 owner decision 1: ONE consent covers the owner. With it on the owner
/// enrolls (their own opt-out flag is ignored); with it off the owner is skipped too.
#[test]
fn owner_enrolls_only_under_the_one_consent() {
    use EnrollConfidence::{OwnerBootstrap, UserConfirmed};
    for confidence in [UserConfirmed, OwnerBootstrap] {
        assert_eq!(
            decide_enrollment(confidence, true, true, true),
            EnrollDecision::EnrollOwner
        );
        assert_eq!(
            decide_enrollment(confidence, true, false, false),
            EnrollDecision::Skip,
            "consent off must stop owner enrollment too"
        );
    }
}

#[test]
fn others_enroll_only_when_consent_on_and_not_opted_out() {
    use EnrollConfidence::UserConfirmed;
    // Consent on + not opted out → enroll.
    assert_eq!(
        decide_enrollment(UserConfirmed, false, true, false),
        EnrollDecision::EnrollPerson
    );
    // Consent OFF → skip (the off-by-default consent posture).
    assert_eq!(
        decide_enrollment(UserConfirmed, false, false, false),
        EnrollDecision::Skip
    );
    // Consent on but this person OPTED OUT → skip (per-person override wins).
    assert_eq!(
        decide_enrollment(UserConfirmed, false, true, true),
        EnrollDecision::Skip
    );
}

/// specs/0039 WS3: an auto-label attribution NEVER enrolls, no matter how permissive the
/// consent is — the guard against the pollution flywheel. And the owner bootstrap is
/// owner-only: it can never grow another person's gallery.
#[test]
fn auto_label_never_enrolls_and_bootstrap_is_owner_only() {
    use EnrollConfidence::{AutoLabel, OwnerBootstrap};
    assert_eq!(
        decide_enrollment(AutoLabel, true, true, false),
        EnrollDecision::Skip
    );
    assert_eq!(
        decide_enrollment(AutoLabel, false, true, false),
        EnrollDecision::Skip
    );
    assert_eq!(
        decide_enrollment(OwnerBootstrap, false, true, false),
        EnrollDecision::Skip
    );
}

// ----- integration: opt-out deletes existing samples (ADR-0007 §6) -----

/// In-memory pool through the app's REAL migration set (the previous hand-rolled DDL
/// had to be kept in sync with four migrations by hand and silently drifted). One
/// connection max — each in-memory connection is a separate database. `foreign_keys`
/// is ON by sqlx default, matching the app pool (manager.rs); the explicit cascades
/// exist because most cross-table links are documentation-only, not declared FKs.
async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn opt_out_deletes_existing_voiceprints_but_keeps_person() {
    let pool = pool_with_schema().await;
    let person = PeopleRepository::create(&pool, "Priya", Some("priya@example.com"), None, None)
        .await
        .unwrap();
    VoiceprintsRepository::add_sample(
        &pool,
        &person.id,
        &[1.0, 0.0],
        "3dspeaker_campplus_sv_en_voxceleb_16k",
        None,
        None,
        Some(1.0),
    )
    .await
    .unwrap();
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, &person.id)
            .await
            .unwrap(),
        1
    );

    // Turn on per-person opt-out → samples deleted, person survives.
    PeopleRepository::set_voiceprint_opt_out(&pool, &person.id, true)
        .await
        .unwrap();
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, &person.id)
            .await
            .unwrap(),
        0,
        "opt-out must delete existing voiceprints"
    );
    assert!(
        PeopleRepository::get(&pool, &person.id)
            .await
            .unwrap()
            .is_some(),
        "the person row must survive opt-out"
    );

    // Flipping opt-out back OFF does NOT recreate samples.
    PeopleRepository::set_voiceprint_opt_out(&pool, &person.id, false)
        .await
        .unwrap();
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, &person.id)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn forget_person_cascades_voiceprints() {
    let pool = pool_with_schema().await;
    let person = PeopleRepository::create(&pool, "Sam", None, None, None)
        .await
        .unwrap();
    VoiceprintsRepository::add_sample(
        &pool,
        &person.id,
        &[0.0, 1.0],
        "3dspeaker_campplus_sv_en_voxceleb_16k",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(PeopleRepository::delete(&pool, &person.id).await.unwrap());
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, &person.id)
            .await
            .unwrap(),
        0,
        "forget-person must cascade-delete voiceprints"
    );
    assert!(PeopleRepository::get(&pool, &person.id)
        .await
        .unwrap()
        .is_none());
}

// ----- specs/0039 WS3: enroll cluster-quality guards -----

/// The `unknown` overflow bucket is a MIX of voices and carries no centroid; the enroll
/// gate must refuse it outright (returns before any embedding/settings work), writing NO
/// voiceprint row. Guards the mixed bucket from ever polluting a gallery.
#[tokio::test]
async fn unknown_bucket_never_enrolls() {
    let pool = pool_with_schema().await;
    let enrolled = enroll_voiceprint_for_speaker(
        &pool,
        "m1",
        crate::diarization::UNKNOWN_SPEAKER_KEY,
        "p-anything",
        EnrollConfidence::UserConfirmed,
    )
    .await
    .unwrap();
    assert!(!enrolled, "the unknown bucket must never enroll");
}

// --- shared setup for the "materially contested" enroll-gate tests (specs/0039 WS3) ---

async fn insert_meeting(pool: &SqlitePool, id: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind("t")
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
}

/// `n` transcript lines `t0..tn` for a meeting under `speaker`.
async fn insert_lines(pool: &SqlitePool, meeting: &str, n: usize, speaker: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    for i in 0..n {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(format!("t{i}"))
        .bind(meeting)
        .bind("hello")
        .bind(&now)
        .bind(speaker)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// A local/mic speaker row carrying a real embedding — the owner-path enroll source.
async fn insert_local_speaker_with_embedding(pool: &SqlitePool, meeting: &str, key: &str) {
    use crate::database::repositories::speaker::SpeakersRepository;
    use crate::diarization::embedding::{embedding_to_bytes, l2_normalize};
    let emb = embedding_to_bytes(&l2_normalize(&[1.0, 0.0, 0.0]));
    SpeakersRepository::upsert(
        pool,
        meeting,
        key,
        "You",
        true, // is_local → owner self-enroll path
        Some(&emb),
        Some(3),
        Some(EMBEDDING_MODEL_ID),
    )
    .await
    .unwrap();
}

/// A single stray override on an otherwise-clean cluster (1/4 = 0.25 < MATERIAL) must NOT
/// block enrollment — the regression the ANY-override gate broke. Routed through the
/// owner/local path with consent on so the enroll actually fires.
#[tokio::test]
async fn single_stray_override_still_enrolls() {
    let pool = pool_with_schema().await;
    insert_meeting(&pool, "m1").await;
    insert_local_speaker_with_embedding(&pool, "m1", crate::diarization::LOCAL_SPEAKER_KEY).await;
    insert_lines(&pool, "m1", 4, crate::diarization::LOCAL_SPEAKER_KEY).await;
    // One stray correction on a single line of the 4-line cluster → 0.25 contested.
    crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository
        ::set(&pool, "m1", "t0", crate::diarization::LOCAL_SPEAKER_KEY)
        .await
        .unwrap();

    let enrolled = enroll_speaker_gated(
        &pool,
        "m1",
        crate::diarization::LOCAL_SPEAKER_KEY,
        "ignored-for-owner-path",
        EnrollConfidence::UserConfirmed,
        false,
        true,
    )
    .await
    .unwrap();
    assert!(
        enrolled,
        "a single stray override must not block a clean cluster's enrollment"
    );
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, OWNER_PERSON_ID)
            .await
            .unwrap(),
        1,
        "the owner gained exactly one sample"
    );
}

/// A MATERIALLY contested cluster (half its lines overridden, 2/4 = 0.5 >= MATERIAL) is
/// refused — its membership was substantially hand-edited, so it is not a clean source.
/// This gate returns before any settings/embedding work, so it is deterministic.
#[tokio::test]
async fn materially_contested_cluster_refuses_enrollment() {
    let pool = pool_with_schema().await;
    insert_meeting(&pool, "m1").await;
    insert_local_speaker_with_embedding(&pool, "m1", crate::diarization::LOCAL_SPEAKER_KEY).await;
    insert_lines(&pool, "m1", 4, crate::diarization::LOCAL_SPEAKER_KEY).await;
    // Override HALF the cluster's lines → materially contested.
    for id in ["t0", "t1"] {
        crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository
            ::set(&pool, "m1", id, crate::diarization::LOCAL_SPEAKER_KEY)
            .await
            .unwrap();
    }

    let enrolled = enroll_speaker_gated(
        &pool,
        "m1",
        crate::diarization::LOCAL_SPEAKER_KEY,
        "ignored",
        EnrollConfidence::UserConfirmed,
        false,
        true,
    )
    .await
    .unwrap();
    assert!(!enrolled, "a materially-contested cluster must not enroll");
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, OWNER_PERSON_ID)
            .await
            .unwrap(),
        0,
        "no sample written when refused"
    );
}

// ----- specs/0078: the owner path under the one consent -----

/// A remote cluster row (`is_local = 0`) with an embedding.
async fn insert_cluster_with_embedding(pool: &SqlitePool, meeting: &str, key: &str) {
    use crate::diarization::embedding::{embedding_to_bytes, l2_normalize};
    let emb = embedding_to_bytes(&l2_normalize(&[0.0, 1.0, 0.0]));
    SpeakersRepository::upsert(
        pool,
        meeting,
        key,
        "Speaker 1",
        false,
        Some(&emb),
        Some(3),
        Some(EMBEDDING_MODEL_ID),
    )
    .await
    .unwrap();
}

/// specs/0078 open question 6: assigning a CLUSTER (not the mic row) to the owner person
/// enrolls under the owner. It used to route on the row's `is_local = 0` and take the
/// others' path, so nothing was written under default settings. With consent off,
/// nothing is written now either.
#[tokio::test]
async fn assigning_a_cluster_to_the_owner_takes_the_owner_path() {
    let pool = pool_with_schema().await;
    insert_meeting(&pool, "m1").await;
    insert_cluster_with_embedding(&pool, "m1", "spk_0").await;

    let off = enroll_speaker_gated(
        &pool,
        "m1",
        "spk_0",
        OWNER_PERSON_ID,
        EnrollConfidence::UserConfirmed,
        false,
        false,
    )
    .await
    .unwrap();
    assert!(!off, "consent off: no owner sample");
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, OWNER_PERSON_ID)
            .await
            .unwrap(),
        0
    );

    let on = enroll_speaker_gated(
        &pool,
        "m1",
        "spk_0",
        OWNER_PERSON_ID,
        EnrollConfidence::UserConfirmed,
        false,
        true,
    )
    .await
    .unwrap();
    assert!(
        on,
        "consent on: the owner person gains the cluster's sample"
    );
    assert_eq!(
        VoiceprintsRepository::count_for_person(&pool, OWNER_PERSON_ID)
            .await
            .unwrap(),
        1
    );
}

/// The owner bootstrap (specs/0078 owner decision 2): gated on the consent, one sample
/// per meeting back-linked to `local`, never re-added after the user retracted it, and
/// never from an auto-label.
#[tokio::test]
async fn owner_bootstrap_adds_one_sample_per_meeting_under_consent() {
    let pool = pool_with_schema().await;
    insert_meeting(&pool, "m1").await;
    let emb = [0.2_f32, 0.9, 0.1];
    let owner_count = |pool: SqlitePool| async move {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM voiceprints WHERE person_id = ?")
            .bind(OWNER_PERSON_ID)
            .fetch_one(&pool)
            .await
            .unwrap()
    };

    use EnrollConfidence::{AutoLabel, OwnerBootstrap};
    assert!(
        !enroll_owner_sample_gated(&pool, "m1", &emb, EMBEDDING_MODEL_ID, OwnerBootstrap, false)
            .await
            .unwrap(),
        "consent off"
    );
    assert!(
        !enroll_owner_sample_gated(&pool, "m1", &emb, EMBEDDING_MODEL_ID, AutoLabel, true)
            .await
            .unwrap(),
        "an auto-label never enrolls"
    );
    assert_eq!(owner_count(pool.clone()).await, 0);

    assert!(
        enroll_owner_sample_gated(&pool, "m1", &emb, EMBEDDING_MODEL_ID, OwnerBootstrap, true)
            .await
            .unwrap()
    );
    let key: Option<String> =
        sqlx::query_scalar("SELECT source_speaker_key FROM voiceprints WHERE person_id = ?")
            .bind(OWNER_PERSON_ID)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(key.as_deref(), Some(crate::diarization::LOCAL_SPEAKER_KEY));

    // A re-run of the pass adds nothing.
    assert!(!enroll_owner_sample_gated(
        &pool,
        "m1",
        &emb,
        EMBEDDING_MODEL_ID,
        OwnerBootstrap,
        true
    )
    .await
    .unwrap());
    // Nor after the user retracted it ("This isn't me" quarantines).
    sqlx::query("UPDATE voiceprints SET quarantined_at = 'now'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!enroll_owner_sample_gated(
        &pool,
        "m1",
        &emb,
        EMBEDDING_MODEL_ID,
        OwnerBootstrap,
        true
    )
    .await
    .unwrap());
    assert_eq!(owner_count(pool.clone()).await, 1);
}
