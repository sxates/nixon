//! Consistent DB snapshot (`VACUUM INTO` + FTS rebuild) and the logical fingerprint the
//! scheduled backup uses to skip unchanged data (specs/0092).

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqliteValueRef};
use sqlx::{ConnectOptions, Connection, Row, SqlitePool, TypeInfo, ValueRef};

use super::manifest::Counts;
use super::BackupError;

/// Tables that describe this machine or this install rather than the user's data. They never
/// travel in the fingerprint (restore keeps the target's own copies).
pub const MACHINE_BOUND_TABLES: &[&str] = &[
    "settings",
    "transcript_settings",
    "summary_prompts_meta",
    "licensing",
    "google_calendar_account",
    "google_calendar_events",
    "google_calendar_sync",
    "_sqlx_migrations",
];

/// What the manifest needs from a snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub schema_version: i64,
    pub counts: Counts,
    /// `(meeting_id, folder_path)` for every meeting.
    pub recordings: Vec<(String, Option<String>)>,
}

async fn open(path: &Path, read_only: bool) -> Result<SqliteConnection, BackupError> {
    let mut opts = SqliteConnectOptions::new()
        .filename(path)
        .foreign_keys(false)
        .read_only(read_only)
        .create_if_missing(false);
    opts = opts.disable_statement_logging();
    Ok(opts.connect().await?)
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Write a consistent copy of the live DB (WAL included) to `dest`, rebuild its FTS indexes
/// (`VACUUM` may renumber rowids of external-content tables) and describe it. `dest` is
/// removed on failure.
pub async fn snapshot_db(pool: &SqlitePool, dest: &Path) -> Result<SnapshotInfo, BackupError> {
    let result = snapshot_inner(pool, dest).await;
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

async fn snapshot_inner(pool: &SqlitePool, dest: &Path) -> Result<SnapshotInfo, BackupError> {
    let _ = std::fs::remove_file(dest);
    sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().into_owned())
        .execute(pool)
        .await?;

    let mut conn = open(dest, false).await?;
    let info = scrub_and_describe(&mut conn).await;
    let closed = conn.close().await;
    let info = info?;
    closed?;
    Ok(info)
}

/// Empty the machine-bound tables (API keys, license key and the like never leave this Mac;
/// `_sqlx_migrations` stays so a restore can migrate), then `VACUUM` in place so the deleted
/// pages are gone from the file, then rebuild FTS and describe.
async fn scrub_and_describe(conn: &mut SqliteConnection) -> Result<SnapshotInfo, BackupError> {
    sqlx::query("PRAGMA secure_delete = ON").execute(&mut *conn).await?;
    for t in MACHINE_BOUND_TABLES.iter().filter(|t| **t != "_sqlx_migrations") {
        let exists: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name = ?")
                .bind(t)
                .fetch_optional(&mut *conn)
                .await?;
        if exists.is_some() {
            sqlx::query(&format!("DELETE FROM {}", quote(t))).execute(&mut *conn).await?;
        }
    }
    sqlx::query("VACUUM").execute(&mut *conn).await?;
    describe_and_rebuild(conn).await
}

async fn describe_and_rebuild(conn: &mut SqliteConnection) -> Result<SnapshotInfo, BackupError> {
    let fts = fts_tables(conn).await?;
    for t in &fts {
        let q = quote(t);
        sqlx::query(&format!("INSERT INTO {q}({q}) VALUES('rebuild')"))
            .execute(&mut *conn)
            .await?;
    }
    let schema_version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
            .fetch_one(&mut *conn)
            .await?;
    let count = |t: &'static str| format!("SELECT COUNT(*) FROM {t}");
    let counts = Counts {
        meetings: sqlx::query_scalar(&count("meetings"))
            .fetch_one(&mut *conn)
            .await?,
        people: sqlx::query_scalar(&count("people"))
            .fetch_one(&mut *conn)
            .await?,
        topics: sqlx::query_scalar(&count("topics"))
            .fetch_one(&mut *conn)
            .await?,
        action_items: sqlx::query_scalar(&count("action_items"))
            .fetch_one(&mut *conn)
            .await?,
    };
    let recordings: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, folder_path FROM meetings ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    Ok(SnapshotInfo {
        schema_version,
        counts,
        recordings,
    })
}

async fn fts_tables(conn: &mut SqliteConnection) -> Result<Vec<String>, BackupError> {
    Ok(sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND sql LIKE '%USING fts5%' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await?)
}

/// Tables the fingerprint covers: not machine-bound, not FTS (or its shadows), not `sqlite_*`.
async fn data_tables(conn: &mut SqliteConnection) -> Result<Vec<String>, BackupError> {
    let fts = fts_tables(conn).await?;
    let all: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(&mut *conn)
            .await?;
    let is_shadow = |n: &str| {
        fts.iter().any(|f| {
            n == f
                || ["_data", "_idx", "_docsize", "_config", "_content"]
                    .iter()
                    .any(|s| n == format!("{f}{s}"))
        })
    };
    Ok(all
        .into_iter()
        .filter(|n| {
            !n.starts_with("sqlite_")
                && !MACHINE_BOUND_TABLES.contains(&n.as_str())
                && !is_shadow(n)
        })
        .collect())
}

fn feed_value(h: &mut Sha256, v: SqliteValueRef<'_>) {
    let ty = v.type_info().name().to_string();
    let bytes: Vec<u8> = if v.is_null() {
        h.update([b'N']);
        return;
    } else {
        match ty.as_str() {
            "INTEGER" | "BOOLEAN" | "DATE" | "TIME" | "DATETIME" | "INT8" => {
                h.update([b'I']);
                <i64 as sqlx::Decode<sqlx::Sqlite>>::decode(v)
                    .unwrap_or_default()
                    .to_le_bytes()
                    .to_vec()
            }
            "REAL" => {
                h.update([b'R']);
                <f64 as sqlx::Decode<sqlx::Sqlite>>::decode(v)
                    .unwrap_or_default()
                    .to_bits()
                    .to_le_bytes()
                    .to_vec()
            }
            "BLOB" => {
                h.update([b'B']);
                <Vec<u8> as sqlx::Decode<sqlx::Sqlite>>::decode(v).unwrap_or_default()
            }
            _ => {
                h.update([b'T']);
                <String as sqlx::Decode<sqlx::Sqlite>>::decode(v)
                    .unwrap_or_default()
                    .into_bytes()
            }
        }
    };
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(&bytes);
}

/// A hash of the user's data: the DB rows (minus machine-bound tables and FTS), the settings
/// files, and each meeting folder's file listing. Stable across snapshots of unchanged data.
pub async fn logical_fingerprint(
    db: &Path,
    settings: &[(PathBuf, String)],
    folders: &[(PathBuf, bool)],
) -> Result<[u8; 32], BackupError> {
    let mut h = Sha256::new();
    let mut conn = open(db, true).await?;
    let rows = hash_tables(&mut conn, &mut h).await;
    let closed = conn.close().await;
    rows?;
    closed?;

    let mut sorted: Vec<&(PathBuf, String)> = settings.iter().collect();
    sorted.sort_by(|a, b| a.1.cmp(&b.1));
    for (path, zip_path) in sorted {
        let bytes = std::fs::read(path)?;
        h.update(b"S");
        h.update((zip_path.len() as u64).to_le_bytes());
        h.update(zip_path.as_bytes());
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(&bytes);
    }

    for (folder, include_audio) in folders {
        h.update(b"F");
        h.update(folder.to_string_lossy().as_bytes());
        let mut entries = Vec::new();
        for p in super::files::folder_entries(folder, *include_audio) {
            let md = std::fs::metadata(&p)?;
            let mtime = md
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            entries.push((name, md.len(), mtime));
        }
        entries.sort();
        for (name, size, mtime) in entries {
            h.update((name.len() as u64).to_le_bytes());
            h.update(name.as_bytes());
            h.update(size.to_le_bytes());
            h.update(mtime.to_le_bytes());
        }
    }
    Ok(h.finalize().into())
}

async fn hash_tables(conn: &mut SqliteConnection, h: &mut Sha256) -> Result<(), BackupError> {
    for table in data_tables(conn).await? {
        let q = quote(&table);
        // pk columns in key order; rowid when the table has none (WITHOUT ROWID tables always do).
        let info = sqlx::query(&format!("PRAGMA table_info({q})"))
            .fetch_all(&mut *conn)
            .await?;
        let mut pk: Vec<(i64, String)> = info
            .iter()
            .filter_map(|r| {
                let n: i64 = r.get("pk");
                (n > 0).then(|| (n, r.get::<String, _>("name")))
            })
            .collect();
        pk.sort();
        let order = if pk.is_empty() {
            "rowid".to_string()
        } else {
            pk.iter()
                .map(|(_, c)| quote(c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        h.update(b"T");
        h.update((table.len() as u64).to_le_bytes());
        h.update(table.as_bytes());
        let rows = sqlx::query(&format!("SELECT * FROM {q} ORDER BY {order}"))
            .fetch_all(&mut *conn)
            .await?;
        for row in &rows {
            h.update(b"R");
            for i in 0..row.columns().len() {
                feed_value(h, row.try_get_raw(i)?);
            }
        }
    }
    Ok(())
}
