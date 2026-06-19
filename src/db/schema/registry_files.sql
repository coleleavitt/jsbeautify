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
