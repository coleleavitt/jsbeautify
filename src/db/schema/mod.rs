//! Project DB schema migrations.
//!
//! All `CREATE TABLE IF NOT EXISTS` statements are idempotent so calling
//! [`init`] multiple times is safe. New columns are added via `ALTER TABLE`
//! guarded by `PRAGMA table_info` checks (matches jxscout's migration style).

use super::ProjectDb;
use rusqlite::params;

const SCHEMA_SQL_BATCHES: &[&str] = &[
    include_str!("registry_files.sql"),
    include_str!("analysis_wordlist.sql"),
    include_str!("workflow_events.sql"),
];

const FTS5_SQL: &str = include_str!("fts5.sql");

/// Initialise the schema (idempotent).
pub(super) fn init(db: &ProjectDb) -> rusqlite::Result<()> {
    db.with_conn(|c| -> rusqlite::Result<()> {
        for sql in SCHEMA_SQL_BATCHES {
            c.execute_batch(sql)?;
        }
        // FTS5 is optional - if the SQLite binary lacks the extension we
        // still want the core schema to load.
        if let Err(e) = c.execute_batch(FTS5_SQL) {
            eprintln!("[db] FTS5 unavailable, skipping word index: {e}");
        }
        Ok(())
    })?;

    // Online migrations: add columns that may not exist on older DBs.
    add_column_if_missing(db, "matches", "seen", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(db, "matches", "is_uninteresting", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(db, "html_files", "ingestion_source", "TEXT NOT NULL DEFAULT 'PROXY'")?;
    add_column_if_missing(db, "js_files", "ingestion_source", "TEXT NOT NULL DEFAULT 'PROXY'")?;
    add_column_if_missing(db, "matches", "provenance", "TEXT NOT NULL DEFAULT 'ast'")?;

    Ok(())
}

fn add_column_if_missing(db: &ProjectDb, table: &str, col: &str, decl: &str) -> rusqlite::Result<()> {
    let exists: bool = db.with_conn(|c| -> rusqlite::Result<bool> {
        let mut stmt = c.prepare(&format!("PRAGMA table_info({table})"))?;
        let cols = stmt.query_map(params![], |r| r.get::<_, String>(1))?;
        for n in cols.flatten() {
            if n == col {
                return Ok(true);
            }
        }
        Ok(false)
    })?;
    if !exists {
        db.with_conn(|c| {
            c.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl}"), [])?;
            Ok::<_, rusqlite::Error>(())
        })?;
    }
    Ok(())
}
