//! DB tests for the "This is me" / "This isn't me" re-keys (specs/0078).

use super::*;

const OWNER: &str = "person-owner-self";

async fn pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn meeting(pool: &SqlitePool, id: &str) {
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, 't', ?, ?)")
        .bind(id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
}

async fn speaker(
    pool: &SqlitePool,
    m: &str,
    key: &str,
    name: &str,
    local: bool,
    emb: Option<&[u8]>,
) {
    SpeakersRepository::upsert(
        pool,
        m,
        key,
        name,
        local,
        emb,
        emb.map(|e| (e.len() / 4) as i64),
        emb.map(|_| "model-x"),
    )
    .await
    .unwrap();
}

async fn line(pool: &SqlitePool, m: &str, id: &str, key: &str) {
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker)
         VALUES (?, ?, 'hi', ?, ?)",
    )
    .bind(id)
    .bind(m)
    .bind(Utc::now().to_rfc3339())
    .bind(key)
    .execute(pool)
    .await
    .unwrap();
}

async fn override_row(pool: &SqlitePool, m: &str, transcript_id: &str, key: &str) {
    sqlx::query(
        "INSERT INTO transcript_speaker_overrides (meeting_id, transcript_id, speaker_key, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(m)
    .bind(transcript_id)
    .bind(key)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
}

async fn sample(pool: &SqlitePool, id: &str, person: &str, m: &str, key: &str) {
    // voiceprints.person_id is a real FK (foreign_keys is on in sqlx's default pool).
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO people (id, display_name, voiceprint_opt_out, created_at, updated_at)
         VALUES (?, 'p', 0, ?, ?)",
    )
    .bind(person)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO voiceprints (id, person_id, embedding, embedding_dim, embedding_model,
                                  source_meeting_id, source_speaker_key, created_at)
         VALUES (?, ?, x'0000803f', 1, 'model-x', ?, ?, ?)",
    )
    .bind(id)
    .bind(person)
    .bind(m)
    .bind(key)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
}

async fn line_key(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT speaker FROM transcripts WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn override_key(pool: &SqlitePool, transcript_id: &str) -> String {
    sqlx::query_scalar(
        "SELECT speaker_key FROM transcript_speaker_overrides WHERE transcript_id = ?",
    )
    .bind(transcript_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// `(source_speaker_key, quarantined)` of one sample.
async fn sample_state(pool: &SqlitePool, id: &str) -> (Option<String>, bool) {
    let (key, q): (Option<String>, i64) = sqlx::query_as(
        "SELECT source_speaker_key, quarantined_at IS NOT NULL FROM voiceprints WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    (key, q != 0)
}

/// `(display_name, is_local, person_id, email, embedding, embedding_model)` of one row.
type Row = (
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
);
async fn row(pool: &SqlitePool, m: &str, key: &str) -> Option<Row> {
    sqlx::query_as(
        "SELECT display_name, is_local, person_id, email, embedding, embedding_model
         FROM speakers WHERE meeting_id = ? AND speaker_key = ?",
    )
    .bind(m)
    .bind(key)
    .fetch_optional(pool)
    .await
    .unwrap()
}

const EMB_A: &[u8] = &[0, 0, 128, 63, 0, 0, 0, 0];
const EMB_B: &[u8] = &[0, 0, 0, 0, 0, 0, 128, 63];

#[tokio::test]
async fn fresh_rekey_moves_the_cluster_onto_local() {
    let pool = pool().await;
    meeting(&pool, "m1").await;
    speaker(&pool, "m1", "spk_0", "Priya", false, Some(EMB_A)).await;
    speaker(&pool, "m1", "spk_1", "Speaker 2", false, Some(EMB_B)).await;
    sqlx::query("UPDATE speakers SET email = 'p@example.com', person_id = 'p-1' WHERE speaker_key = 'spk_0'")
        .execute(&pool)
        .await
        .unwrap();
    line(&pool, "m1", "t0", "spk_0").await;
    line(&pool, "m1", "t1", "spk_0").await;
    line(&pool, "m1", "t2", "spk_1").await;
    override_row(&pool, "m1", "t1", "spk_0").await;
    override_row(&pool, "m1", "t2", "spk_1").await;
    sample(&pool, "vp-other", "p-1", "m1", "spk_0").await;
    sample(&pool, "vp-owner", OWNER, "m1", "spk_0").await;

    let out = SpeakersRepository::rekey_to_local(&pool, "m1", "spk_0", OWNER, true)
        .await
        .unwrap()
        .expect("the cluster exists");
    assert_eq!(
        out,
        RekeyToLocal {
            moved_lines: 2,
            merged_into_existing: false,
            quarantined_other_samples: 1,
        }
    );

    assert_eq!(line_key(&pool, "t0").await.as_deref(), Some("local"));
    assert_eq!(line_key(&pool, "t1").await.as_deref(), Some("local"));
    assert_eq!(
        line_key(&pool, "t2").await.as_deref(),
        Some("spk_1"),
        "other speakers untouched"
    );
    assert_eq!(
        override_key(&pool, "t1").await,
        "local",
        "override re-pointed"
    );
    assert_eq!(override_key(&pool, "t2").await, "spk_1");

    let (name, is_local, person, email, emb, model) = row(&pool, "m1", "local").await.unwrap();
    assert_eq!(name, "You");
    assert_eq!(is_local, 1);
    assert_eq!(person.as_deref(), Some(OWNER));
    assert_eq!(email, None, "someone else's address no longer applies");
    assert_eq!(emb.as_deref(), Some(EMB_A), "the embedding is kept");
    assert_eq!(model.as_deref(), Some("model-x"));
    assert!(row(&pool, "m1", "spk_0").await.is_none());

    assert_eq!(
        sample_state(&pool, "vp-other").await,
        (Some("local".into()), true)
    );
    assert_eq!(
        sample_state(&pool, "vp-owner").await,
        (Some("local".into()), false)
    );
}

#[tokio::test]
async fn rekey_merges_into_an_existing_local_row() {
    let pool = pool().await;
    // m1: `local` exists with no embedding (lines were reassigned to "You") and a rename.
    meeting(&pool, "m1").await;
    speaker(&pool, "m1", "local", "Me", true, None).await;
    speaker(&pool, "m1", "spk_0", "Speaker 1", false, Some(EMB_A)).await;
    line(&pool, "m1", "t0", "local").await;
    line(&pool, "m1", "t1", "spk_0").await;
    // m2: `local` already carries its own embedding, which must win.
    meeting(&pool, "m2").await;
    speaker(&pool, "m2", "local", "You", true, Some(EMB_B)).await;
    speaker(&pool, "m2", "spk_0", "Speaker 1", false, Some(EMB_A)).await;

    let out = SpeakersRepository::rekey_to_local(&pool, "m1", "spk_0", OWNER, true)
        .await
        .unwrap()
        .unwrap();
    assert!(out.merged_into_existing);
    assert_eq!(out.moved_lines, 1);
    assert_eq!(line_key(&pool, "t1").await.as_deref(), Some("local"));
    let (name, is_local, person, _, emb, model) = row(&pool, "m1", "local").await.unwrap();
    assert_eq!(name, "Me", "the owner's rename is kept");
    assert_eq!(is_local, 1);
    assert_eq!(person.as_deref(), Some(OWNER));
    assert_eq!(
        emb.as_deref(),
        Some(EMB_A),
        "an empty local takes the cluster's embedding"
    );
    assert_eq!(model.as_deref(), Some("model-x"));
    assert!(
        row(&pool, "m1", "spk_0").await.is_none(),
        "the cluster row is folded away"
    );

    // An automatic owner label never replaces local's own embedding.
    SpeakersRepository::rekey_to_local(&pool, "m2", "spk_0", OWNER, false)
        .await
        .unwrap()
        .unwrap();
    let (_, _, _, _, emb, _) = row(&pool, "m2", "local").await.unwrap();
    assert_eq!(
        emb.as_deref(),
        Some(EMB_B),
        "local's own embedding is not replaced"
    );
}

/// "This is me" into an automatic `local` (a cluster the owner's voiceprint or the
/// carry-over picked): the voice the user just confirmed becomes local's embedding. Into
/// a `local` the user confirmed earlier, the earlier confirmation stands.
#[tokio::test]
async fn a_confirmed_merge_replaces_an_automatic_locals_embedding() {
    let pool = pool().await;
    for m in ["auto", "confirmed"] {
        meeting(&pool, m).await;
        speaker(&pool, m, "local", "You", true, Some(EMB_B)).await;
        speaker(&pool, m, "spk_0", "Speaker 1", false, Some(EMB_A)).await;
    }
    sqlx::query("UPDATE meetings SET owner_label = 'confirmed' WHERE id = 'confirmed'")
        .execute(&pool)
        .await
        .unwrap();

    for m in ["auto", "confirmed"] {
        let out = SpeakersRepository::rekey_to_local(&pool, m, "spk_0", OWNER, true)
            .await
            .unwrap()
            .unwrap();
        assert!(out.merged_into_existing);
        assert!(row(&pool, m, "spk_0").await.is_none());
        let label: Option<String> =
            sqlx::query_scalar("SELECT owner_label FROM meetings WHERE id = ?")
                .bind(m)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(label.as_deref(), Some("confirmed"), "{m}");
    }
    let (_, _, person, _, emb, _) = row(&pool, "auto", "local").await.unwrap();
    assert_eq!(person.as_deref(), Some(OWNER));
    assert_eq!(emb.as_deref(), Some(EMB_A), "the confirmed voice wins");
    let (_, _, _, _, emb, _) = row(&pool, "confirmed", "local").await.unwrap();
    assert_eq!(
        emb.as_deref(),
        Some(EMB_B),
        "an earlier confirmation stands"
    );
}

#[tokio::test]
async fn rekey_of_a_missing_speaker_changes_nothing() {
    let pool = pool().await;
    meeting(&pool, "m1").await;
    line(&pool, "m1", "t0", "spk_9").await;
    assert_eq!(
        SpeakersRepository::rekey_to_local(&pool, "m1", "spk_9", OWNER, true)
            .await
            .unwrap(),
        None
    );
    assert_eq!(line_key(&pool, "t0").await.as_deref(), Some("spk_9"));
    assert_eq!(
        SpeakersRepository::rekey_to_local(&pool, "m1", "local", OWNER, true)
            .await
            .unwrap(),
        None,
        "local onto itself is refused"
    );
}

#[tokio::test]
async fn unmark_moves_local_to_the_next_free_key_and_quarantines_owner_samples() {
    let pool = pool().await;
    meeting(&pool, "m1").await;
    meeting(&pool, "m2").await;
    speaker(&pool, "m1", "local", "You", true, Some(EMB_A)).await;
    speaker(&pool, "m1", "spk_0", "Speaker 1", false, Some(EMB_B)).await;
    sqlx::query("UPDATE speakers SET person_id = ? WHERE speaker_key = 'local'")
        .bind(OWNER)
        .execute(&pool)
        .await
        .unwrap();
    line(&pool, "m1", "t0", "local").await;
    line(&pool, "m1", "t1", "spk_0").await;
    // A stale override still names spk_3, so the new key must skip past it.
    line(&pool, "m1", "t2", "local").await;
    override_row(&pool, "m1", "t2", "local").await;
    override_row(&pool, "m1", "t9", "spk_3").await;
    sample(&pool, "vp-m1", OWNER, "m1", "local").await;
    sample(&pool, "vp-m2", OWNER, "m2", "local").await;

    let out = SpeakersRepository::rekey_from_local(&pool, "m1", OWNER)
        .await
        .unwrap()
        .expect("m1 has a local row");
    assert_eq!(
        out,
        RekeyFromLocal {
            new_key: "spk_4".into(),
            moved_lines: 1,
            kept_lines: 1,
            quarantined_owner_samples: 1,
        }
    );
    assert_eq!(line_key(&pool, "t0").await.as_deref(), Some("spk_4"));
    assert_eq!(line_key(&pool, "t1").await.as_deref(), Some("spk_0"));
    // The user pinned t2 to "You" by hand: the line and its override stay theirs, and a
    // fresh "You" row (no embedding) keeps it named.
    assert_eq!(line_key(&pool, "t2").await.as_deref(), Some("local"));
    assert_eq!(override_key(&pool, "t2").await, "local");
    let (name, is_local, person, _, emb, _) = row(&pool, "m1", "local").await.unwrap();
    assert_eq!(
        (name.as_str(), is_local, person, emb),
        ("You", 1, None, None)
    );
    let label: Option<String> =
        sqlx::query_scalar("SELECT owner_label FROM meetings WHERE id = 'm1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(label.as_deref(), Some("rejected"));
    let (name, is_local, person, _, emb, _) = row(&pool, "m1", "spk_4").await.unwrap();
    assert_eq!(name, "Speaker 5");
    assert_eq!(is_local, 0);
    assert_eq!(person, None);
    assert_eq!(
        emb.as_deref(),
        Some(EMB_A),
        "the embedding stays with the voice"
    );

    // Quarantined, and still back-linked to local (see rekey_from_local's docs).
    assert_eq!(
        sample_state(&pool, "vp-m1").await,
        (Some("local".into()), true)
    );
    assert_eq!(
        sample_state(&pool, "vp-m2").await,
        (Some("local".into()), false),
        "another meeting's owner sample is untouched"
    );
}

/// Without hand-pinned lines, "This isn't me" leaves no `local` row behind.
#[tokio::test]
async fn unmark_without_pinned_lines_leaves_no_local_row() {
    let pool = pool().await;
    meeting(&pool, "m1").await;
    speaker(&pool, "m1", "local", "You", true, Some(EMB_A)).await;
    line(&pool, "m1", "t0", "local").await;
    let out = SpeakersRepository::rekey_from_local(&pool, "m1", OWNER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((out.moved_lines, out.kept_lines), (1, 0));
    assert!(row(&pool, "m1", "local").await.is_none());
    assert_eq!(
        SpeakersRepository::rekey_from_local(&pool, "m1", OWNER)
            .await
            .unwrap(),
        None,
        "no local row left"
    );
}

#[test]
fn next_free_index_skips_every_used_key() {
    let keys = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(next_free_cluster_index(&keys(&[])), 0);
    assert_eq!(
        next_free_cluster_index(&keys(&["local", "unknown", "manual_x"])),
        0
    );
    assert_eq!(
        next_free_cluster_index(&keys(&["spk_0", "spk_10", "spk_2"])),
        11
    );
}
