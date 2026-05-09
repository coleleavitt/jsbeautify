//! Project DB schema migrations.
//!
//! All `CREATE TABLE IF NOT EXISTS` statements are idempotent so calling
//! [`init`] multiple times is safe. New columns are added via `ALTER TABLE`
//! guarded by `PRAGMA table_info` checks (matches jxscout's migration style).

use super::ProjectDb;
use rusqlite::params;

const SCHEMA_SQL: &str = r#"
-- ── Project registry ────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS projects (
    name              TEXT PRIMARY KEY,
    working_directory TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

-- ── Captured files ──────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS js_files (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    url               TEXT NOT NULL,
    fs_path           TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    request_headers   TEXT,
    is_history_file   INTEGER NOT NULL DEFAULT 0,
    ingestion_source  TEXT NOT NULL DEFAULT 'PROXY',
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_js_files_url        ON js_files(url);
CREATE INDEX IF NOT EXISTS idx_js_files_fs_path    ON js_files(fs_path);
CREATE INDEX IF NOT EXISTS idx_js_files_history    ON js_files(is_history_file);

CREATE TABLE IF NOT EXISTS html_files (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    url               TEXT NOT NULL,
    fs_path           TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    request_headers   TEXT,
    is_history_file   INTEGER NOT NULL DEFAULT 0,
    ingestion_source  TEXT NOT NULL DEFAULT 'PROXY',
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_html_files_url      ON html_files(url);
CREATE INDEX IF NOT EXISTS idx_html_files_fs_path  ON html_files(fs_path);
CREATE INDEX IF NOT EXISTS idx_html_files_history  ON html_files(is_history_file);

CREATE TABLE IF NOT EXISTS html_files_inline_js (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    html_file_id      INTEGER NOT NULL REFERENCES html_files(id) ON DELETE CASCADE,
    js_file_id        INTEGER NOT NULL REFERENCES js_files(id)   ON DELETE CASCADE,
    block_index       INTEGER NOT NULL,
    line              INTEGER NOT NULL,
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_html_files_inline_js_html_file_id ON html_files_inline_js(html_file_id);

-- ── Relationship graph ──────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS js_loader_relationships (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_url        TEXT NOT NULL,
    page_file_path  TEXT NOT NULL,
    js_url          TEXT NOT NULL,
    js_file_path    TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_js_loader_page_url        ON js_loader_relationships(page_url);
CREATE INDEX IF NOT EXISTS idx_js_loader_page_file_path  ON js_loader_relationships(page_file_path);
CREATE INDEX IF NOT EXISTS idx_js_loader_js_url          ON js_loader_relationships(js_url);
CREATE INDEX IF NOT EXISTS idx_js_loader_js_file_path    ON js_loader_relationships(js_file_path);

CREATE TABLE IF NOT EXISTS iframe_loader_relationships (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    page_url            TEXT NOT NULL,
    page_file_path      TEXT NOT NULL,
    iframe_url          TEXT NOT NULL,
    iframe_file_path    TEXT NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_iframe_loader_page_url       ON iframe_loader_relationships(page_url);
CREATE INDEX IF NOT EXISTS idx_iframe_loader_page_file_path ON iframe_loader_relationships(page_file_path);
CREATE INDEX IF NOT EXISTS idx_iframe_loader_iframe_url     ON iframe_loader_relationships(iframe_url);
CREATE INDEX IF NOT EXISTS idx_iframe_loader_iframe_file_path ON iframe_loader_relationships(iframe_file_path);

CREATE TABLE IF NOT EXISTS discovered_lazy_loaded_js_files (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_js_id    INTEGER NOT NULL REFERENCES js_files(id) ON DELETE CASCADE,
    chunk_url       TEXT NOT NULL,
    discovered_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_lazy_loaded_parent ON discovered_lazy_loaded_js_files(parent_js_id);

CREATE TABLE IF NOT EXISTS source_map_files (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    js_file_id      INTEGER NOT NULL REFERENCES js_files(id) ON DELETE CASCADE,
    url             TEXT NOT NULL,
    fs_path         TEXT NOT NULL,
    content_hash    TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_source_map_files_js_file_id ON source_map_files(js_file_id);

CREATE TABLE IF NOT EXISTS source_map_reversed_files (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    source_map_file_id  INTEGER NOT NULL REFERENCES source_map_files(id) ON DELETE CASCADE,
    source_path         TEXT NOT NULL,
    reversed_fs_path    TEXT NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_smrf_source_map_file_id ON source_map_reversed_files(source_map_file_id);

CREATE TABLE IF NOT EXISTS untracked_urls (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    url          TEXT NOT NULL UNIQUE,
    reason       TEXT,
    seen_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Analysis ─────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS analysis_run (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    analyzer_name   TEXT NOT NULL,
    file_type       TEXT NOT NULL,        -- 'js' | 'html' | 'reversed_source'
    file_id         INTEGER NOT NULL,
    started_at      TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at     TEXT
);
CREATE INDEX IF NOT EXISTS idx_analysis_run_analyzer       ON analysis_run(analyzer_name);
CREATE INDEX IF NOT EXISTS idx_analysis_run_file_type      ON analysis_run(file_type);
CREATE INDEX IF NOT EXISTS idx_analysis_run_file_type_id   ON analysis_run(file_type, file_id);

CREATE TABLE IF NOT EXISTS matches (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    analysis_run_id   INTEGER NOT NULL REFERENCES analysis_run(id) ON DELETE CASCADE,
    match_kind        TEXT NOT NULL,
    match_value       TEXT NOT NULL,
    severity          TEXT NOT NULL DEFAULT 'info',
    provenance        TEXT NOT NULL DEFAULT 'ast',
    start_line        INTEGER,
    start_col         INTEGER,
    end_line          INTEGER,
    end_col           INTEGER,
    metadata          TEXT,
    seen              INTEGER NOT NULL DEFAULT 0,
    is_uninteresting  INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_matches_analysis_run_id          ON matches(analysis_run_id);
CREATE INDEX IF NOT EXISTS idx_matches_match_kind               ON matches(match_kind);
CREATE INDEX IF NOT EXISTS idx_matches_match_kind_match_value   ON matches(match_kind, match_value);
CREATE INDEX IF NOT EXISTS idx_matches_seen                     ON matches(seen, analysis_run_id);
CREATE INDEX IF NOT EXISTS idx_matches_is_uninteresting         ON matches(is_uninteresting, analysis_run_id);

CREATE TABLE IF NOT EXISTS findings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    severity        TEXT NOT NULL,
    kind            TEXT NOT NULL,
    description     TEXT NOT NULL,
    dedup_key       TEXT NOT NULL,
    metadata        TEXT,
    metadata_hash   TEXT NOT NULL,
    found_at        TEXT NOT NULL DEFAULT (datetime('now')),
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(kind, dedup_key)
);
CREATE INDEX IF NOT EXISTS idx_findings_kind                    ON findings(kind);
CREATE INDEX IF NOT EXISTS idx_findings_severity                ON findings(severity);
CREATE INDEX IF NOT EXISTS idx_findings_severity_kind           ON findings(severity, kind);
CREATE INDEX IF NOT EXISTS idx_findings_kind_metadata_hash      ON findings(kind, metadata_hash);

-- ── Wordlist ────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS unique_words (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    word            TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS idx_unique_words_word ON unique_words(word);

CREATE TABLE IF NOT EXISTS word_occurrences (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    word_id         INTEGER NOT NULL REFERENCES unique_words(id) ON DELETE CASCADE,
    file_type       TEXT NOT NULL,
    file_id         INTEGER NOT NULL,
    source          TEXT NOT NULL,    -- js_variable_name, html_body_text, etc
    count           INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_word_occurrences_word_id          ON word_occurrences(word_id);
CREATE INDEX IF NOT EXISTS idx_word_occurrences_file_type        ON word_occurrences(file_type);
CREATE INDEX IF NOT EXISTS idx_word_occurrences_file_id_type     ON word_occurrences(file_type, file_id);
CREATE INDEX IF NOT EXISTS idx_word_occurrences_source           ON word_occurrences(source);
CREATE INDEX IF NOT EXISTS idx_word_occurrences_source_file_type ON word_occurrences(source, file_type);

-- ── Triage / workflow ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS journal_entries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_type      TEXT NOT NULL,           -- test | hypothesis | observation | recon
    target          TEXT NOT NULL,
    description     TEXT NOT NULL,
    tags            TEXT,                    -- comma-separated
    status          TEXT NOT NULL DEFAULT 'untested',  -- untested | confirmed | disproved
    evidence        TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_journal_entry_type           ON journal_entries(entry_type);
CREATE INDEX IF NOT EXISTS idx_journal_status               ON journal_entries(status);
CREATE INDEX IF NOT EXISTS idx_journal_target               ON journal_entries(target);
CREATE INDEX IF NOT EXISTS idx_journal_entry_type_status    ON journal_entries(entry_type, status);

CREATE TABLE IF NOT EXISTS bookmark_groups (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL UNIQUE,
    highlight_color TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS bookmarks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id        INTEGER NOT NULL REFERENCES bookmark_groups(id) ON DELETE CASCADE,
    file_path       TEXT NOT NULL,
    file_type       TEXT NOT NULL DEFAULT 'js',
    start_line      INTEGER NOT NULL,
    start_col       INTEGER NOT NULL DEFAULT 0,
    end_line        INTEGER NOT NULL,
    end_col         INTEGER NOT NULL DEFAULT 0,
    note            TEXT,
    match_id        INTEGER,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_bookmarks_file_path ON bookmarks(file_path);
CREATE INDEX IF NOT EXISTS idx_bookmarks_group_id  ON bookmarks(group_id);
CREATE INDEX IF NOT EXISTS idx_bookmarks_match_id  ON bookmarks(match_id);

-- ── Monitoring ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS monitors (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT NOT NULL,
    cron_expression     TEXT NOT NULL,
    file_types          TEXT,           -- json array of types
    source_type         TEXT NOT NULL DEFAULT 'url',  -- url | script
    source_script       TEXT,           -- only when source_type=script
    enabled             INTEGER NOT NULL DEFAULT 1,
    last_run_at         TEXT,
    next_run_at         TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_monitors_enabled ON monitors(enabled);

CREATE TABLE IF NOT EXISTS monitor_urls (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    monitor_id      INTEGER NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
    url_pattern     TEXT NOT NULL,
    is_wildcard     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_monitor_urls_monitor_id ON monitor_urls(monitor_id);

CREATE TABLE IF NOT EXISTS monitoring_failed_urls (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    monitor_id      INTEGER NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
    url             TEXT NOT NULL,
    error           TEXT NOT NULL,
    failed_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Event bus (persistent) ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    topic           TEXT NOT NULL,
    payload         TEXT NOT NULL,         -- JSON
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_events_topic       ON events(topic);
CREATE INDEX IF NOT EXISTS idx_events_created_at  ON events(created_at);

CREATE TABLE IF NOT EXISTS subscribers (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    topic           TEXT NOT NULL,
    queue_name      TEXT NOT NULL UNIQUE,
    concurrency     INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_subscribers_topic       ON subscribers(topic);
CREATE INDEX IF NOT EXISTS idx_subscribers_queue_name  ON subscribers(queue_name);

CREATE TABLE IF NOT EXISTS subscriber_jobs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    subscriber      TEXT NOT NULL,
    event_id        INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'pending',   -- pending | running | done | failed | killed
    attempts        INTEGER NOT NULL DEFAULT 0,
    run_at          TEXT NOT NULL DEFAULT (datetime('now')),
    locked_at       TEXT,
    heartbeat_at    TEXT,
    error           TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_event_id      ON subscriber_jobs(event_id);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_subscriber    ON subscriber_jobs(subscriber);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_status_run_at ON subscriber_jobs(status, run_at);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_running       ON subscriber_jobs(subscriber, locked_at);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_heartbeat     ON subscriber_jobs(subscriber, heartbeat_at);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_status_subscriber_event_id
    ON subscriber_jobs(status, subscriber, event_id);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_subscriber_status_event_id
    ON subscriber_jobs(subscriber, status, event_id);
CREATE INDEX IF NOT EXISTS idx_sub_jobs_subscriber_status_run_at
    ON subscriber_jobs(subscriber, status, run_at);
"#;

const FTS5_SQL: &str = r#"
-- FTS5 virtual table for fast word search. Falls back gracefully if FTS5 is
-- unavailable (rusqlite's bundled SQLite includes it by default).
CREATE VIRTUAL TABLE IF NOT EXISTS unique_words_fts USING fts5(
    word,
    content='unique_words',
    content_rowid='id'
);
CREATE TRIGGER IF NOT EXISTS unique_words_ai AFTER INSERT ON unique_words BEGIN
    INSERT INTO unique_words_fts(rowid, word) VALUES (new.id, new.word);
END;
CREATE TRIGGER IF NOT EXISTS unique_words_ad AFTER DELETE ON unique_words BEGIN
    INSERT INTO unique_words_fts(unique_words_fts, rowid, word) VALUES('delete', old.id, old.word);
END;
CREATE TRIGGER IF NOT EXISTS unique_words_au AFTER UPDATE ON unique_words BEGIN
    INSERT INTO unique_words_fts(unique_words_fts, rowid, word) VALUES('delete', old.id, old.word);
    INSERT INTO unique_words_fts(rowid, word) VALUES (new.id, new.word);
END;
"#;

/// Initialise the schema (idempotent).
pub(super) fn init(db: &ProjectDb) -> rusqlite::Result<()> {
    db.with_conn(|c| -> rusqlite::Result<()> {
        c.execute_batch(SCHEMA_SQL)?;
        // FTS5 is optional — if the SQLite binary lacks the extension we
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
