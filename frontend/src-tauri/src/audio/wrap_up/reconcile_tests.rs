use super::*;
use crate::database::manager::DatabaseManager;

async fn db(dir: &Path) -> DatabaseManager {
    DatabaseManager::new(
        dir.join("t.sqlite").to_str().unwrap(),
        dir.join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap()
}

async fn meeting(pool: &SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at) \
         VALUES (?, 'Standup', '2026-11-05T10:00:00Z', '2026-11-05T10:00:00Z')",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

async fn rows(pool: &SqlitePool, id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn folder(root: &Path, name: &str, meeting_id: &str, status: &str, texts: &[&str]) -> PathBuf {
    let f = root.join(name);
    std::fs::create_dir_all(&f).unwrap();
    std::fs::write(
        f.join("metadata.json"),
        serde_json::json!({ "meeting_id": meeting_id, "status": status }).to_string(),
    )
    .unwrap();
    let segments: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            serde_json::json!({
                "id": format!("seg_{i}"), "text": t, "audio_start_time": i as f64,
                "audio_end_time": i as f64 + 1.0, "duration": 1.0, "display_time": "[00:00]",
            })
        })
        .collect();
    std::fs::write(
        f.join("transcripts.json"),
        serde_json::json!({ "segments": segments }).to_string(),
    )
    .unwrap();
    f
}

#[tokio::test]
async fn imports_only_completed_unsaved_sessions_and_never_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let db = db(dir.path()).await;
    let pool = db.pool();
    let root = dir.path().join("recordings");

    meeting(pool, "stranded").await;
    let stranded = folder(&root, "a", "stranded", "completed", &["hello", "world"]);
    meeting(pool, "interrupted").await;
    folder(&root, "b", "interrupted", "recording", &["mid recording"]);
    meeting(pool, "saved").await;
    folder(&root, "c", "saved", "completed", &["one", "two", "three"]);
    sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES ('t1', 'saved', 'one', '')")
        .execute(pool)
        .await
        .unwrap();
    meeting(pool, "blank").await;
    folder(&root, "d", "blank", "completed", &["  "]);

    assert_eq!(import_unsaved_sessions(pool, std::slice::from_ref(&root)).await, 1);
    assert_eq!(rows(pool, "stranded").await, 2);
    assert_eq!(
        rows(pool, "interrupted").await,
        0,
        "a mid-recording folder is the resume prompt's"
    );
    assert_eq!(
        rows(pool, "saved").await,
        1,
        "a saved meeting is never touched"
    );
    assert_eq!(rows(pool, "blank").await, 0);
    let linked: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = 'stranded'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(linked.as_deref(), Some(stranded.to_string_lossy().as_ref()));

    assert_eq!(
        import_unsaved_sessions(pool, &[root]).await,
        0,
        "second launch"
    );
    assert_eq!(rows(pool, "stranded").await, 2);
}
