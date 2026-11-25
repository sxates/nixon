//! specs/0092 task 2: DB snapshot (VACUUM INTO + FTS rebuild) and logical fingerprint.

mod common;

use std::path::Path;

use app_lib::backup::manifest::build_schema_version;
use app_lib::backup::snapshot::{logical_fingerprint, snapshot_db};
use common::fresh_db;
use sqlx::{Connection, SqliteConnection, SqlitePool};

async fn add_meeting(pool: &SqlitePool, id: &str, text: &str) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) \
         VALUES (?, ?, 't', 't', ?)",
    )
    .bind(id)
    .bind(format!("title {id}"))
    .bind(format!("/f/{id}"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, 't')",
    )
    .bind(format!("tr-{id}"))
    .bind(id)
    .bind(text)
    .execute(pool)
    .await
    .unwrap();
}

async fn open(p: &Path) -> SqliteConnection {
    SqliteConnection::connect(&format!("sqlite://{}", p.display()))
        .await
        .unwrap()
}

#[tokio::test]
async fn snapshot_includes_uncheckpointed_wal_rows() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "hello").await;
    let dest = dir.path().join("snap.sqlite");
    snapshot_db(db.pool(), &dest).await.unwrap();
    let mut c = open(&dest).await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings WHERE id='m1'")
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let ic: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut c)
        .await
        .unwrap();
    assert_eq!(ic, "ok");
}

#[tokio::test]
async fn snapshot_fts_finds_rows_after_vacuum() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "alphaword").await;
    add_meeting(db.pool(), "m2", "betaword").await;
    sqlx::query("DELETE FROM meetings WHERE id='m1'")
        .execute(db.pool())
        .await
        .unwrap();
    let dest = dir.path().join("snap.sqlite");
    snapshot_db(db.pool(), &dest).await.unwrap();
    let mut c = open(&dest).await;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT meeting_id FROM transcripts_fts WHERE transcripts_fts MATCH 'betaword'",
    )
    .fetch_all(&mut c)
    .await
    .unwrap();
    assert_eq!(ids, vec!["m2".to_string()]);
}

#[tokio::test]
async fn snapshot_counts_and_schema() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "a").await;
    add_meeting(db.pool(), "m2", "b").await;
    let dest = dir.path().join("snap.sqlite");
    let info = snapshot_db(db.pool(), &dest).await.unwrap();
    assert_eq!(info.counts.meetings, 2);
    assert_eq!(info.counts.people, 0);
    assert_eq!(info.counts.topics, 0);
    assert_eq!(info.counts.action_items, 0);
    assert_eq!(info.schema_version, build_schema_version());
    let mut rec = info.recordings.clone();
    rec.sort();
    assert_eq!(
        rec,
        vec![
            ("m1".to_string(), Some("/f/m1".to_string())),
            ("m2".to_string(), Some("/f/m2".to_string()))
        ]
    );
}

#[tokio::test]
async fn snapshot_failure_leaves_no_dest() {
    let (dir, db) = fresh_db().await;
    let dest = dir.path().join("no-such-dir").join("snap.sqlite");
    assert!(snapshot_db(db.pool(), &dest).await.is_err());
    assert!(!dest.exists());
}

async fn fp(dir: &Path, db: &SqlitePool, name: &str) -> [u8; 32] {
    let dest = dir.join(name);
    snapshot_db(db, &dest).await.unwrap();
    logical_fingerprint(&dest, &[], &[]).await.unwrap()
}

#[tokio::test]
async fn fingerprint_stable_across_snapshots() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "a").await;
    let a = fp(dir.path(), db.pool(), "a.sqlite").await;
    let b = fp(dir.path(), db.pool(), "b.sqlite").await;
    assert_eq!(a, b);
}

#[tokio::test]
async fn fingerprint_changes_on_edit() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "a").await;
    let a = fp(dir.path(), db.pool(), "a.sqlite").await;
    sqlx::query("UPDATE transcripts SET transcript='changed' WHERE id='tr-m1'")
        .execute(db.pool())
        .await
        .unwrap();
    let b = fp(dir.path(), db.pool(), "b.sqlite").await;
    assert_ne!(a, b);
}

#[tokio::test]
async fn fingerprint_ignores_machine_bound_tables() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "a").await;
    let a = fp(dir.path(), db.pool(), "a.sqlite").await;
    sqlx::query("UPDATE settings SET provider='other-provider'")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO settings (id, provider, model, whisperModel) VALUES ('zz','p','m','w') ON CONFLICT DO NOTHING")
        .execute(db.pool())
        .await
        .unwrap();
    let b = fp(dir.path(), db.pool(), "b.sqlite").await;
    assert_eq!(a, b);
}

#[tokio::test]
async fn fingerprint_covers_settings_files_and_folders() {
    let (dir, db) = fresh_db().await;
    let snap = dir.path().join("a.sqlite");
    snapshot_db(db.pool(), &snap).await.unwrap();
    let s = dir.path().join("prefs.json");
    std::fs::write(&s, "{}").unwrap();
    let folder = dir.path().join("mf");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("transcript.json"), "x").unwrap();
    let settings = vec![(s.clone(), "preferences.json".to_string())];
    let folders = vec![(folder.clone(), false)];
    let a = logical_fingerprint(&snap, &settings, &folders)
        .await
        .unwrap();
    std::fs::write(&s, "{\"a\":1}").unwrap();
    let b = logical_fingerprint(&snap, &settings, &folders)
        .await
        .unwrap();
    assert_ne!(a, b);
    std::fs::write(folder.join("transcript.json"), "xyz").unwrap();
    let c = logical_fingerprint(&snap, &settings, &folders)
        .await
        .unwrap();
    assert_ne!(b, c);
}

#[tokio::test]
async fn snapshot_rebuilds_a_desynced_fts_index() {
    let (dir, db) = fresh_db().await;
    add_meeting(db.pool(), "m1", "realword").await;
    // Desync the index from its content table, bypassing the triggers: drop every indexed
    // entry, then plant a stale one for text no transcript holds.
    sqlx::query("INSERT INTO transcripts_fts(transcripts_fts) VALUES('delete-all')")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO transcripts_fts(rowid, transcript, meeting_id) VALUES (9999, 'ghostword', 'ghost')")
        .execute(db.pool())
        .await
        .unwrap();
    let dest = dir.path().join("snap.sqlite");
    snapshot_db(db.pool(), &dest).await.unwrap();
    let mut c = open(&dest).await;
    let real: Vec<String> = sqlx::query_scalar(
        "SELECT meeting_id FROM transcripts_fts WHERE transcripts_fts MATCH 'realword'",
    )
    .fetch_all(&mut c)
    .await
    .unwrap();
    assert_eq!(real, vec!["m1".to_string()], "rebuild re-indexes the content table");
    let ghost: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transcripts_fts WHERE transcripts_fts MATCH 'ghostword'",
    )
    .fetch_one(&mut c)
    .await
    .unwrap();
    assert_eq!(ghost, 0, "rebuild drops stale index entries");
}
