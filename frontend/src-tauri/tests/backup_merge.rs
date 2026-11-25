//! specs/0092 task 6: Add-what's-missing import. The hard rule: nothing already on this Mac is
//! edited or overwritten.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use app_lib::backup::add::{import_add, ADD_RECORDINGS_STAGING_NAME, ADD_STAGING_DIR_NAME};
use app_lib::backup::export::{export_to, ExportOptions};
use app_lib::backup::merge::{MergeCounts, TableClass, CLASSIFICATION};
use app_lib::backup::progress::NoProgress;
use app_lib::backup::replace::MARKER_NAME;
use app_lib::backup::BackupError;
use common::fresh_db;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

struct Profile {
    _dir: tempfile::TempDir,
    db: app_lib::database::manager::DatabaseManager,
    app_data: PathBuf,
    root: PathBuf,
    _base: tempfile::TempDir,
}

async fn profile() -> Profile {
    let (dir, db) = fresh_db().await;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("rec");
    fs::create_dir_all(&root).unwrap();
    Profile {
        app_data: dir.path().to_path_buf(),
        _dir: dir,
        db,
        root,
        _base: base,
    }
}

fn make_folder(root: &Path, name: &str) -> PathBuf {
    let f = root.join(name);
    fs::create_dir_all(&f).unwrap();
    fs::write(f.join("metadata.json"), b"{\"a\":1}").unwrap();
    fs::write(f.join("audio.opus"), vec![7u8; 1024]).unwrap();
    f
}

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sql)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn add_meeting(pool: &SqlitePool, id: &str, folder: Option<&Path>) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, 't', 't', ?)",
    )
    .bind(id)
    .bind(format!("title {id}"))
    .bind(folder.map(|p| p.to_string_lossy().into_owned()))
    .execute(pool)
    .await
    .unwrap();
}

async fn add_transcript(pool: &SqlitePool, id: &str, meeting: &str, text: &str) {
    sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp) VALUES (?, ?, ?, 't')")
        .bind(id)
        .bind(meeting)
        .bind(text)
        .execute(pool)
        .await
        .unwrap();
}

async fn add_todo(pool: &SqlitePool, id: &str, sort: i64) {
    sqlx::query(
        "INSERT INTO action_items (id, description, source, content_key, sort_order, created_at, updated_at) \
         VALUES (?, 'x', 'manual', ?, ?, 't', 't')",
    )
    .bind(id)
    .bind(id)
    .bind(sort)
    .execute(pool)
    .await
    .unwrap();
}

async fn add_topic(pool: &SqlitePool, id: &str, sort: i64) {
    sqlx::query("INSERT INTO topics (id, name, sort_order, created_at, updated_at) VALUES (?, ?, ?, 't', 't')")
        .bind(id)
        .bind(id)
        .bind(sort)
        .execute(pool)
        .await
        .unwrap();
}

async fn count(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// Every data row of every non-FTS table, as text, for before/after comparisons.
async fn dump_where(pool: &SqlitePool, filter: Option<(&str, &str)>) -> String {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '%_fts%' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = String::new();
    for t in tables {
        let cols: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(&t)
                .fetch_all(pool)
                .await
                .unwrap();
        let has_meeting_id = cols.iter().any(|c| c == "meeting_id");
        let quoted: Vec<String> = cols.iter().map(|c| format!("quote(\"{c}\")")).collect();
        let (where_sql, bind) = match filter {
            Some((col, val)) => {
                let key = if t == "meetings" { "id" } else { col };
                if (t == "meetings" && col == "meeting_id") || (t != "meetings" && has_meeting_id) {
                    (format!(" WHERE {key} = ?"), Some(val))
                } else {
                    continue;
                }
            }
            None => (String::new(), None),
        };
        let sql = format!(
            "SELECT {} FROM \"{t}\"{where_sql} ORDER BY rowid",
            quoted.join(" || '|' || ")
        );
        let mut q = sqlx::query_scalar::<_, String>(&sql);
        if let Some(v) = bind {
            q = q.bind(v);
        }
        for row in q.fetch_all(pool).await.unwrap() {
            out.push_str(&format!("{t}: {row}\n"));
        }
    }
    out
}

async fn dump(pool: &SqlitePool) -> String {
    dump_where(pool, None).await
}

async fn export(a: &Profile, out: &Path) -> PathBuf {
    let opts = ExportOptions {
        include_audio: true,
        app_data_dir: a.app_data.clone(),
        abort_if: None,
        allowed_roots: vec![],
    };
    export_to(a.db.pool(), out, &opts, &NoProgress, &CancellationToken::new())
        .await
        .unwrap()
        .path
}

async fn add_into(b: &Profile, zip: &Path) -> Result<MergeCounts, BackupError> {
    import_add(
        b.db.pool(),
        zip,
        &b.app_data,
        &b.root,
        &NoProgress,
        &CancellationToken::new(),
    )
    .await
}

/// Backup source: two meetings with folders, transcripts, notes, a to-do, a topic, a person.
async fn old_mac() -> Profile {
    let a = profile().await;
    let p = a.db.pool();
    add_meeting(p, "m1", Some(&make_folder(&a.root, "Alpha_1"))).await;
    add_meeting(p, "m2", Some(&make_folder(&a.root, "Beta_2"))).await;
    add_transcript(p, "ta1", "m1", "hello from alpha").await;
    add_transcript(p, "ta2", "m2", "a zebra crossed the road").await;
    exec(p, "INSERT INTO meeting_notes (meeting_id, notes_markdown, created_at, updated_at) VALUES ('m2','notes two','t','t')").await;
    exec(p, "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, created_at, updated_at) VALUES ('sp2','m2','local','You','t','t')").await;
    exec(p, "INSERT INTO action_items (id, meeting_id, description, content_key, created_at, updated_at) VALUES ('ai1','m2','do it','k','t','t')").await;
    exec(p, "INSERT INTO topics (id, name, created_at, updated_at) VALUES ('t1','Topic','t','t')").await;
    exec(p, "INSERT INTO people (id, display_name, created_at, updated_at) VALUES ('p1','Pat','t','t')").await;
    exec(p, "INSERT INTO voiceprints (id, person_id, embedding, embedding_dim, embedding_model, created_at) VALUES ('vp1','p1',x'00',1,'m','t')").await;
    a
}

async fn local_mac() -> Profile {
    let b = profile().await;
    let p = b.db.pool();
    add_meeting(p, "b1", Some(&make_folder(&b.root, "Local_1"))).await;
    add_transcript(p, "tb1", "b1", "local words").await;
    exec(p, "DELETE FROM settings").await;
    exec(p, "INSERT INTO settings (id, provider, model, whisperModel) VALUES ('b-settings','b-prov','m','w')").await;
    b
}

#[tokio::test]
async fn classification_covers_every_table() {
    let (_dir, db) = fresh_db().await;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert!(!tables.is_empty());
    for t in &tables {
        assert!(
            CLASSIFICATION.iter().any(|(n, _)| n == t),
            "table `{t}` is missing from merge::CLASSIFICATION"
        );
    }
    for (n, c) in CLASSIFICATION {
        assert!(tables.iter().any(|t| t == n), "classification names unknown table `{n}`");
        if n.contains("_fts") {
            assert_eq!(*c, TableClass::Skip, "{n}");
        }
    }
    for m in app_lib::backup::MACHINE_BOUND_TABLES {
        assert!(CLASSIFICATION.iter().any(|(n, c)| n == m && *c == TableClass::MachineBound), "{m}");
    }
}

#[tokio::test]
async fn add_into_empty_profile_imports_all() {
    let a = old_mac().await;
    let b = profile().await;
    let settings_before = dump_settings(b.db.pool()).await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;

    let counts = add_into(&b, &zip).await.unwrap();
    assert_eq!(counts.meetings, 2);
    assert_eq!(counts.people, 1);
    assert_eq!(counts.topics, 1);
    assert_eq!(counts.action_items, 1);
    assert_eq!(counts.already_here, 0);
    let mut ids = counts.new_meeting_ids.clone();
    ids.sort();
    assert_eq!(ids, ["m1", "m2"]);

    let p = b.db.pool();
    assert_eq!(count(p, "SELECT COUNT(*) FROM transcripts").await, 2);
    assert_eq!(count(p, "SELECT COUNT(*) FROM meeting_notes").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM speakers").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM voiceprints").await, 1);
    let folder: String = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id='m1'")
        .fetch_one(p)
        .await
        .unwrap();
    assert!(folder.starts_with(b.root.to_str().unwrap()), "{folder}");
    assert!(Path::new(&folder).join("audio.opus").is_file());
    assert_eq!(dump_settings(p).await, settings_before, "settings are this Mac's");
    assert!(!b.app_data.join(ADD_STAGING_DIR_NAME).exists());
    assert!(!b.root.join(ADD_RECORDINGS_STAGING_NAME).exists());
}

async fn dump_settings(pool: &SqlitePool) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT COALESCE(group_concat(quote(id) || quote(provider), ','), '') FROM settings",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn add_overlapping_adds_only_new_and_changes_nothing() {
    let a = old_mac().await;
    let b = local_mac().await;
    // B has m1 too, edited locally, with its own (different) child rows.
    let p = b.db.pool();
    add_meeting(p, "m1", Some(&make_folder(&b.root, "Mine_1"))).await;
    exec(p, "UPDATE meetings SET title = 'edited locally' WHERE id = 'm1'").await;
    add_transcript(p, "tb-m1", "m1", "my own words").await;
    exec(p, "INSERT INTO meeting_notes (meeting_id, notes_markdown, created_at, updated_at) VALUES ('m1','local notes','t','t')").await;
    let m1_before = dump_where(p, Some(("meeting_id", "m1"))).await;
    assert!(m1_before.contains("edited locally"));
    let b1_before = dump_where(p, Some(("meeting_id", "b1"))).await;

    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;
    let counts = add_into(&b, &zip).await.unwrap();
    assert_eq!(counts.meetings, 1);
    assert_eq!(counts.new_meeting_ids, ["m2"]);
    assert_eq!(counts.already_here, 1);

    assert_eq!(dump_where(p, Some(("meeting_id", "m1"))).await, m1_before);
    assert_eq!(dump_where(p, Some(("meeting_id", "b1"))).await, b1_before);
    // m2 and its children arrived.
    assert_eq!(count(p, "SELECT COUNT(*) FROM transcripts WHERE meeting_id='m2'").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM meeting_notes WHERE meeting_id='m2'").await, 1);
    assert_eq!(count(p, "SELECT COUNT(*) FROM speakers WHERE meeting_id='m2'").await, 1);
}

#[tokio::test]
async fn add_same_backup_twice_is_noop() {
    let a = old_mac().await;
    let b = local_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;
    add_into(&b, &zip).await.unwrap();
    let before = dump(b.db.pool()).await;

    let again = add_into(&b, &zip).await.unwrap();
    assert_eq!(
        (again.meetings, again.people, again.topics, again.action_items),
        (0, 0, 0, 0)
    );
    assert!(again.new_meeting_ids.is_empty());
    assert_eq!(again.already_here, 2);
    assert_eq!(dump(b.db.pool()).await, before);
    // No stray duplicate folders either.
    let mut names: Vec<String> = fs::read_dir(&b.root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["Alpha_1", "Beta_2", "Local_1"]);
}

#[tokio::test]
async fn add_skips_orphans_instead_of_failing() {
    let a = old_mac().await;
    let p = a.db.pool();
    exec(p, "INSERT INTO action_items (id, description, assignee_person_id, content_key, source, created_at, updated_at) VALUES ('orph1','x','ghost-person','o1','manual','t','t')").await;
    exec(p, "INSERT INTO action_items (id, description, topic_id, content_key, source, created_at, updated_at) VALUES ('orph2','x','ghost-topic','o2','manual','t','t')").await;
    exec(p, "INSERT INTO action_item_people (action_item_id, person_id) VALUES ('ai1','ghost-person')").await;
    exec(p, "INSERT INTO topic_entries (id, topic_id, kind, occurred_at, body_json, created_at) VALUES ('te-orph','ghost-topic','k','t','{}','t')").await;
    let b = local_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;

    let counts = add_into(&b, &zip).await.unwrap();
    assert_eq!(counts.action_items, 1, "only ai1 is importable");
    let bp = b.db.pool();
    assert_eq!(count(bp, "SELECT COUNT(*) FROM action_items WHERE id LIKE 'orph%'").await, 0);
    assert_eq!(count(bp, "SELECT COUNT(*) FROM action_item_people").await, 0);
    assert_eq!(count(bp, "SELECT COUNT(*) FROM topic_entries").await, 0);
}

#[tokio::test]
async fn add_appends_sort_order() {
    let a = profile().await;
    add_todo(a.db.pool(), "a1", 0).await;
    add_todo(a.db.pool(), "a2", 1).await;
    add_topic(a.db.pool(), "ta1", 0).await;
    add_topic(a.db.pool(), "ta2", 1).await;
    let b = profile().await;
    for (i, id) in ["b1", "b2", "b3"].iter().enumerate() {
        add_todo(b.db.pool(), id, i as i64).await;
    }
    add_topic(b.db.pool(), "tb1", 0).await;
    add_topic(b.db.pool(), "tb2", 1).await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;

    let counts = add_into(&b, &zip).await.unwrap();
    assert_eq!((counts.action_items, counts.topics), (2, 2));
    let p = b.db.pool();
    let todos: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, sort_order FROM action_items ORDER BY sort_order")
            .fetch_all(p)
            .await
            .unwrap();
    let todos: Vec<(&str, i64)> = todos.iter().map(|(i, s)| (i.as_str(), *s)).collect();
    assert_eq!(todos, [("b1", 0), ("b2", 1), ("b3", 2), ("a1", 3), ("a2", 4)]);
    let topics: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, sort_order FROM topics ORDER BY sort_order")
            .fetch_all(p)
            .await
            .unwrap();
    let topics: Vec<(&str, i64)> = topics.iter().map(|(i, s)| (i.as_str(), *s)).collect();
    assert_eq!(topics, [("tb1", 0), ("tb2", 1), ("ta1", 2), ("ta2", 3)]);
}

#[tokio::test]
async fn add_imported_meeting_is_searchable() {
    let a = old_mac().await;
    let b = local_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;
    add_into(&b, &zip).await.unwrap();
    let hits: Vec<String> =
        sqlx::query_scalar("SELECT meeting_id FROM transcripts_fts WHERE transcripts_fts MATCH 'zebra'")
            .fetch_all(b.db.pool())
            .await
            .unwrap();
    assert_eq!(hits, ["m2"]);
    let notes: Vec<String> =
        sqlx::query_scalar("SELECT meeting_id FROM meeting_notes_fts WHERE meeting_notes_fts MATCH 'two'")
            .fetch_all(b.db.pool())
            .await
            .unwrap();
    assert_eq!(notes, ["m2"]);
}

#[tokio::test]
async fn add_places_folders_only_for_new_meetings() {
    let a = old_mac().await;
    let b = local_mac().await;
    add_meeting(b.db.pool(), "m1", Some(&make_folder(&b.root, "Mine_1"))).await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;
    add_into(&b, &zip).await.unwrap();

    assert!(b.root.join("Beta_2/audio.opus").is_file());
    assert!(!b.root.join("Alpha_1").exists(), "m1 already existed here");
    let p = b.db.pool();
    let m2: String = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id='m2'")
        .fetch_one(p)
        .await
        .unwrap();
    assert_eq!(m2, b.root.join("Beta_2").to_string_lossy());
    let m1: String = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id='m1'")
        .fetch_one(p)
        .await
        .unwrap();
    assert_eq!(m1, b.root.join("Mine_1").to_string_lossy());
}

fn assert_clean(b: &Profile) {
    assert!(!b.app_data.join(ADD_STAGING_DIR_NAME).exists(), "staging left behind");
    assert!(!b.root.join(ADD_RECORDINGS_STAGING_NAME).exists(), "recording staging left behind");
}

#[tokio::test]
async fn add_failure_leaves_no_half_import() {
    let a = old_mac().await;
    let b = local_mac().await;
    let before = dump(b.db.pool()).await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;

    fs::set_permissions(&b.root, fs::Permissions::from_mode(0o555)).unwrap();
    let res = add_into(&b, &zip).await;
    fs::set_permissions(&b.root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(res.is_err());
    assert_eq!(dump(b.db.pool()).await, before);
    assert_clean(&b);
}

#[tokio::test]
async fn add_failure_after_placing_folders_rolls_back_and_undoes() {
    let a = old_mac().await;
    let b = local_mac().await;
    let p = b.db.pool();
    // The folder_path rewrite (after the folders are placed, before COMMIT) blows up.
    exec(p, "CREATE TRIGGER boom BEFORE UPDATE OF folder_path ON meetings BEGIN SELECT RAISE(ABORT, 'boom'); END").await;
    let before = dump(p).await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;

    let err = add_into(&b, &zip).await.unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
    assert_eq!(dump(p).await, before);
    assert!(!b.root.join("Alpha_1").exists());
    assert!(!b.root.join("Beta_2").exists());
    assert_clean(&b);

    // And the connection is usable again: drop the trigger, import works.
    exec(p, "DROP TRIGGER boom").await;
    assert_eq!(add_into(&b, &zip).await.unwrap().meetings, 2);
}

#[tokio::test]
async fn add_refuses_while_a_replace_is_pending() {
    let a = old_mac().await;
    let b = local_mac().await;
    fs::write(b.app_data.join(MARKER_NAME), b"{}").unwrap();
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;
    assert_eq!(add_into(&b, &zip).await.unwrap_err(), BackupError::Busy);
    assert_eq!(count(b.db.pool(), "SELECT COUNT(*) FROM meetings").await, 1);
}

#[tokio::test]
async fn detach_failure_after_commit_keeps_the_import() {
    let a = old_mac().await;
    let b = local_mac().await;
    let out = tempfile::tempdir().unwrap();
    let zip = export(&a, out.path()).await;
    let frag = b.app_data.to_string_lossy().into_owned();
    app_lib::backup::merge::FAIL_DETACH_FOR_PATHS.lock().unwrap().push(frag.clone());
    let res = add_into(&b, &zip).await;
    app_lib::backup::merge::FAIL_DETACH_FOR_PATHS.lock().unwrap().retain(|f| f != &frag);

    let counts = res.expect("a detach failure after commit must not fail the import");
    assert_eq!(counts.meetings, 2);
    let p = b.db.pool();
    assert_eq!(count(p, "SELECT COUNT(*) FROM meetings").await, 3);
    for (id, name) in [("m1", "Alpha_1"), ("m2", "Beta_2")] {
        assert!(b.root.join(name).join("audio.opus").is_file(), "{name} stays in the root");
        let fp: String = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(id)
            .fetch_one(p)
            .await
            .unwrap();
        assert_eq!(fp, b.root.join(name).to_string_lossy());
    }
    assert_clean(&b);
}
