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
