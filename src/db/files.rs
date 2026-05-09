//! `js_files` / `html_files` repository with content-hash-based history.
//!
//! Implements the "if hash changed, mark old as history and insert new" pattern
//! from REPORT_06 §10 (Complete Ingestion Flow).

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};

use super::ProjectDb;

/// Where a file came from. Mirrors jxscout's `ingestion_source` column values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IngestionSource {
    Proxy,
    FsIngest,
    Cli,
}

impl IngestionSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proxy => "PROXY",
            Self::FsIngest => "FS_INGEST",
            Self::Cli => "CLI",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: i64,
    pub url: String,
    pub fs_path: String,
    pub content_hash: String,
    pub is_history_file: bool,
}

/// Insert (or update with history) a JS file. Returns the new file's id.
///
/// If a non-history row exists for `url`:
///   - If `content_hash` matches → return existing id, no-op.
///   - If different → mark existing as history, insert fresh row.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn upsert_js_file(
    db: &ProjectDb,
    url: &str,
    fs_path: &str,
    content_hash: &str,
    request_headers: Option<&str>,
    ingestion: IngestionSource,
) -> Result<i64> {
    upsert_file_impl(db, "js_files", url, fs_path, content_hash, request_headers, ingestion)
}

/// Same as [`upsert_js_file`] for the `html_files` table.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn upsert_html_file(
    db: &ProjectDb,
    url: &str,
    fs_path: &str,
    content_hash: &str,
    request_headers: Option<&str>,
    ingestion: IngestionSource,
) -> Result<i64> {
    upsert_file_impl(db, "html_files", url, fs_path, content_hash, request_headers, ingestion)
}

fn upsert_file_impl(
    db: &ProjectDb,
    table: &str,
    url: &str,
    fs_path: &str,
    content_hash: &str,
    request_headers: Option<&str>,
    ingestion: IngestionSource,
) -> Result<i64> {
    db.with_conn(|c| {
        // Look for existing non-history row for this URL.
        let existing: Option<(i64, String)> = c
            .query_row(
                &format!(
                    "SELECT id, content_hash FROM {table} WHERE url = ?1 AND is_history_file = 0 \
                     ORDER BY id DESC LIMIT 1"
                ),
                params![url],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();

        if let Some((id, prev_hash)) = existing {
            if prev_hash == content_hash {
                return Ok(id);
            }
            // Mark old as history.
            c.execute(
                &format!("UPDATE {table} SET is_history_file = 1, updated_at = datetime('now') WHERE id = ?1"),
                params![id],
            )?;
        }

        // Insert new active row.
        c.execute(
            &format!(
                "INSERT INTO {table} (url, fs_path, content_hash, request_headers, ingestion_source, \
                 is_history_file, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, datetime('now'), datetime('now'))"
            ),
            params![url, fs_path, content_hash, request_headers, ingestion.as_str()],
        )?;
        Ok(c.last_insert_rowid())
    })
}

/// Look up the current (non-history) row for `url`.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn current_js_file(db: &ProjectDb, url: &str) -> Result<Option<FileRecord>> {
    current_file_impl(db, "js_files", url)
}

/// Look up the current (non-history) row for `url`.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn current_html_file(db: &ProjectDb, url: &str) -> Result<Option<FileRecord>> {
    current_file_impl(db, "html_files", url)
}

fn current_file_impl(db: &ProjectDb, table: &str, url: &str) -> Result<Option<FileRecord>> {
    db.with_conn(|c| {
        let row = c
            .query_row(
                &format!(
                    "SELECT id, url, fs_path, content_hash, is_history_file FROM {table} \
                     WHERE url = ?1 AND is_history_file = 0 ORDER BY id DESC LIMIT 1"
                ),
                params![url],
                |r| {
                    Ok(FileRecord {
                        id: r.get(0)?,
                        url: r.get(1)?,
                        fs_path: r.get(2)?,
                        content_hash: r.get(3)?,
                        is_history_file: r.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .ok();
        Ok(row)
    })
}

/// Get the full history (newest first) of a file by URL.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn js_file_history(db: &ProjectDb, url: &str) -> Result<Vec<FileRecord>> {
    file_history_impl(db, "js_files", url)
}

/// Get the full history (newest first) of a file by URL.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn html_file_history(db: &ProjectDb, url: &str) -> Result<Vec<FileRecord>> {
    file_history_impl(db, "html_files", url)
}

fn file_history_impl(db: &ProjectDb, table: &str, url: &str) -> Result<Vec<FileRecord>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT id, url, fs_path, content_hash, is_history_file FROM {table} \
             WHERE url = ?1 ORDER BY id DESC"
        ))?;
        let rows = stmt.query_map(params![url], |r| {
            Ok(FileRecord {
                id: r.get(0)?,
                url: r.get(1)?,
                fs_path: r.get(2)?,
                content_hash: r.get(3)?,
                is_history_file: r.get::<_, i64>(4)? != 0,
            })
        })?;
        Ok(rows.flatten().collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_returns_same_id_when_hash_unchanged() {
        let db = ProjectDb::open_in_memory().unwrap();
        let id1 = upsert_js_file(&db, "https://x/a.js", "/p/a.js", "h1", None, IngestionSource::Proxy).unwrap();
        let id2 = upsert_js_file(&db, "https://x/a.js", "/p/a.js", "h1", None, IngestionSource::Proxy).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn upsert_marks_history_on_hash_change() {
        let db = ProjectDb::open_in_memory().unwrap();
        let id_v1 = upsert_js_file(&db, "https://x/a.js", "/p/a.js", "h1", None, IngestionSource::Proxy).unwrap();
        let id_v2 = upsert_js_file(&db, "https://x/a.js", "/p/a.js", "h2", None, IngestionSource::Proxy).unwrap();
        assert_ne!(id_v1, id_v2);

        let hist = js_file_history(&db, "https://x/a.js").unwrap();
        assert_eq!(hist.len(), 2);
        // newest first
        assert_eq!(hist[0].id, id_v2);
        assert!(!hist[0].is_history_file);
        assert!(hist[1].is_history_file);
    }

    #[test]
    fn current_returns_only_non_history_row() {
        let db = ProjectDb::open_in_memory().unwrap();
        upsert_js_file(&db, "https://x/a.js", "/p/a.js", "h1", None, IngestionSource::Proxy).unwrap();
        let id_v2 = upsert_js_file(&db, "https://x/a.js", "/p/a.js", "h2", None, IngestionSource::Proxy).unwrap();

        let cur = current_js_file(&db, "https://x/a.js").unwrap().unwrap();
        assert_eq!(cur.id, id_v2);
        assert_eq!(cur.content_hash, "h2");
    }
}
