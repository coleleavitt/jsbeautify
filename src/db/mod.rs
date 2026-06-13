//! SQLite-backed project database.
//!
//! Ports `crates/core/project-db/` and parts of `crates/core/db/` from
//! jxscout-pro-v2. Schema mirrors REPORT_02 / REPORT_05 / REPORT_06.
//!
//! ## Tables
//!
//! Core ingestion:
//!   - `projects` — project registry (name, working_directory, timestamps)
//!   - `html_files` / `js_files` — captured source files with content hash
//!   - `html_files_inline_js` — inline `<script>` blocks
//!   - `js_loader_relationships` / `iframe_loader_relationships` — page→asset edges
//!   - `discovered_lazy_loaded_js_files` — webpack/Next.js dynamic chunks
//!   - `source_map_files` / `source_map_reversed_files` — sourcemap chain
//!   - `untracked_urls` — URLs seen but not in scope
//!
//! Analysis:
//!   - `analysis_run` — one row per (file, analyzer) invocation
//!   - `matches` — per-finding rows with seen/uninteresting flags
//!   - `findings` — promoted matches with severity + dedup_key
//!   - `unique_words` / `word_occurrences` — wordlist DB (FTS5-backed)
//!
//! Triage / workflow:
//!   - `journal_entries` — test/hypothesis tracking
//!   - `bookmarks` / `bookmark_groups` — code bookmarks
//!
//! Monitoring:
//!   - `monitors` / `monitor_urls` / `monitoring_failed_urls`
//!
//! Event bus:
//!   - `events` / `subscribers` / `subscriber_jobs` (lease-based queue)
//!
//! Pragmas: WAL mode, NORMAL synchronous, foreign_keys ON, busy_timeout 5000.

use rusqlite::{Connection, OpenFlags, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub mod bookmarks;
pub mod event_bus;
pub mod files;
pub mod journal;
pub mod matches;
pub mod monitors;
pub mod relationships;
pub mod schema;
pub mod words;

/// One project DB. Wraps a `rusqlite::Connection` behind a Mutex so it can be
/// shared across analyzer/monitor/proxy threads. SQLite's WAL mode supports
/// concurrent readers natively; a single writer serialised through the mutex
/// is the safe baseline.
#[derive(Clone)]
pub struct ProjectDb {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl ProjectDb {
    /// Open or create a project DB at `path`. Initializes schema on first run.
    ///
    /// # Errors
    /// Returns the underlying rusqlite error on connection or schema failure.
    pub fn open<P: AsRef<Path>>(path: P) -> rusqlite::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open_with_flags(
            &path_buf,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;

        // Pragmas mirror jxscout's startup config.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -64000;",
        )?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: path_buf,
        };
        schema::init(&db)?;
        Ok(db)
    }

    /// In-memory DB for tests.
    ///
    /// # Errors
    /// Returns the underlying rusqlite error on schema failure.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        };
        schema::init(&db)?;
        Ok(db)
    }

    /// Run a closure with the locked connection. The lock is held only for
    /// the duration of the closure.
    pub fn with_conn<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T,
    {
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        f(&g)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Convenience: register a project name -> working dir mapping.
    ///
    /// # Errors
    /// Returns rusqlite errors from the underlying execute.
    pub fn upsert_project(&self, name: &str, working_dir: &str) -> rusqlite::Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO projects (name, working_directory, created_at, updated_at)
                 VALUES (?1, ?2, datetime('now'), datetime('now'))
                 ON CONFLICT(name) DO UPDATE SET working_directory = excluded.working_directory,
                     updated_at = datetime('now')",
                params![name, working_dir],
            )?;
            Ok(())
        })
    }

    /// Sha-256 hex of `bytes`. Used for content_hash columns.
    #[must_use]
    pub fn content_hash(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        hex::encode(h.finalize())
    }
}

/// Return `~/.jsbeautify/projects/<name>/project.db`.
#[must_use]
pub fn default_project_db_path(project_name: &str) -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".jsbeautify")
        .join("projects")
        .join(project_name)
        .join("project.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_memory_and_creates_schema() {
        let db = ProjectDb::open_in_memory().unwrap();
        // Sanity: querying one of the tables shouldn't error.
        db.with_conn(|c| {
            let n: i64 = c.query_row("SELECT count(*) FROM projects", [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0);
        });
    }

    #[test]
    fn upsert_project_idempotent() {
        let db = ProjectDb::open_in_memory().unwrap();
        db.upsert_project("acme", "/tmp/acme").unwrap();
        db.upsert_project("acme", "/tmp/acme-v2").unwrap();
        db.with_conn(|c| {
            let dir: String = c
                .query_row("SELECT working_directory FROM projects WHERE name = 'acme'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(dir, "/tmp/acme-v2");
        });
    }

    #[test]
    fn with_conn_recovers_from_poisoned_lock() {
        let db = ProjectDb::open_in_memory().unwrap();

        // A panic inside a `with_conn` closure poisons the connection mutex.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            db.with_conn(|_c| panic!("intentional panic inside with_conn"));
        }));
        assert!(result.is_err(), "closure panic should propagate");
        assert!(db.conn.lock().is_err(), "mutex should now be poisoned");

        // The next caller must still get a working connection (graceful
        // recovery via `unwrap_or_else(|e| e.into_inner())`) rather than
        // cascading the panic.
        db.upsert_project("after", "/tmp/after").unwrap();
        let dir: String = db.with_conn(|c| {
            c.query_row("SELECT working_directory FROM projects WHERE name = 'after'", [], |r| {
                r.get(0)
            })
            .unwrap()
        });
        assert_eq!(dir, "/tmp/after");
    }
}
