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
