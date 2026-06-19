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
