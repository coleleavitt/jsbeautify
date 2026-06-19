//! Wordlist DB layer with FTS5-backed search.
//!
//! `unique_words` is the canonical word table; `word_occurrences` is the join
//! table to `(file_type, file_id, source)` with an occurrence count.
//!
//! `unique_words_fts` is a FTS5 virtual table that mirrors `unique_words.word`,
//! enabling `MATCH` queries for fast prefix/substring lookups when implementing
//! `--word-prefix` / interactive search.

use rusqlite::{Result, params};
use std::fmt::Write as _;

use super::ProjectDb;

/// Insert a (word, `file_type`, `file_id`, source) occurrence. If the word
/// already exists the count is incremented.
///
/// # Errors
pub fn record(db: &ProjectDb, word: &str, file_type: &str, file_id: i64, source: &str) -> Result<()> {
    db.with_conn(|c| {
        // Find or create unique_words.id
        c.execute("INSERT OR IGNORE INTO unique_words (word) VALUES (?1)", params![word])?;
        let word_id: i64 = c.query_row("SELECT id FROM unique_words WHERE word = ?1", params![word], |r| {
            r.get(0)
        })?;

        // Upsert occurrence
        let updated = c.execute(
            "UPDATE word_occurrences SET count = count + 1
             WHERE word_id = ?1 AND file_type = ?2 AND file_id = ?3 AND source = ?4",
            params![word_id, file_type, file_id, source],
        )?;
        if updated == 0 {
            c.execute(
                "INSERT INTO word_occurrences (word_id, file_type, file_id, source, count)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                params![word_id, file_type, file_id, source],
            )?;
        }
        Ok(())
    })
}

#[derive(Debug, Default, Clone)]
pub struct WordQuery<'a> {
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub source: Option<&'a str>,
    pub file_type: Option<&'a str>,
    pub limit: Option<usize>,
    pub sort_by_count: bool,
    pub fts_match: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct WordRow {
    pub word: String,
    pub total_count: i64,
}

/// Fetch unique words, optionally filtered + sorted.
///
/// # Errors
pub fn list(db: &ProjectDb, q: &WordQuery<'_>) -> Result<Vec<WordRow>> {
    let mut sql = String::from(
        "SELECT uw.word, COALESCE(SUM(wo.count), 0) AS total
         FROM unique_words uw
         LEFT JOIN word_occurrences wo ON wo.word_id = uw.id WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(s) = q.source {
        sql.push_str(" AND wo.source = ?");
        args.push(Box::new(s.to_string()));
    }
    if let Some(t) = q.file_type {
        sql.push_str(" AND wo.file_type = ?");
        args.push(Box::new(t.to_string()));
    }
    if let Some(min) = q.min_length {
        sql.push_str(" AND length(uw.word) >= ?");
        args.push(Box::new(i64::try_from(min).unwrap_or(i64::MAX)));
    }
    if let Some(max) = q.max_length
        && max > 0
    {
        sql.push_str(" AND length(uw.word) <= ?");
        args.push(Box::new(i64::try_from(max).unwrap_or(i64::MAX)));
    }
    if let Some(m) = q.fts_match {
        // Use FTS5 if available; if not we fallback to LIKE %
        sql.push_str(" AND uw.id IN (SELECT rowid FROM unique_words_fts WHERE word MATCH ?)");
        args.push(Box::new(m.to_string()));
    }
    sql.push_str(" GROUP BY uw.id");
    if q.sort_by_count {
        sql.push_str(" ORDER BY total DESC, uw.word ASC");
    } else {
        sql.push_str(" ORDER BY uw.word ASC");
    }
    if let Some(l) = q.limit {
        write!(sql, " LIMIT {l}").ok();
    }

    db.with_conn(|c| {
        let mut stmt = match c.prepare(&sql) {
            Ok(s) => s,
            Err(_) if q.fts_match.is_some() => {
                // FTS5 likely unavailable; retry without the MATCH clause.
                let fallback = sql.replace(
                    " AND uw.id IN (SELECT rowid FROM unique_words_fts WHERE word MATCH ?)",
                    " AND uw.word LIKE ?",
                );
                c.prepare(&fallback)?
            }
            Err(e) => return Err(e),
        };
        let p_dyn: Vec<&dyn rusqlite::ToSql> = args.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(p_dyn.iter().copied()), |r| {
            Ok(WordRow {
                word: r.get(0)?,
                total_count: r.get(1)?,
            })
        })?;
        Ok(rows.flatten().collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_query() {
        let db = ProjectDb::open_in_memory().unwrap();
        record(&db, "admin", "js", 1, "js_variable_name").unwrap();
        record(&db, "admin", "js", 1, "js_variable_name").unwrap(); // increment
        record(&db, "user", "js", 1, "js_variable_name").unwrap();
        record(&db, "login", "html", 2, "html_body_text").unwrap();

        let alpha = list(&db, &WordQuery::default()).unwrap();
        let names: Vec<&str> = alpha.iter().map(|r| r.word.as_str()).collect();
        assert!(names.contains(&"admin"));
        assert!(names.contains(&"login"));

        let by_count = list(
            &db,
            &WordQuery {
                sort_by_count: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_count[0].word, "admin");
        assert_eq!(by_count[0].total_count, 2);

        let only_js = list(
            &db,
            &WordQuery {
                file_type: Some("js"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!only_js.iter().any(|r| r.word == "login"));

        let limited = list(
            &db,
            &WordQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn fts_or_like_fallback() {
        let db = ProjectDb::open_in_memory().unwrap();
        record(&db, "admin", "js", 1, "v").unwrap();
        record(&db, "administrator", "js", 1, "v").unwrap();
        record(&db, "user", "js", 1, "v").unwrap();

        let m = list(
            &db,
            &WordQuery {
                fts_match: Some("admin*"),
                ..Default::default()
            },
        )
        .unwrap();
        // Either FTS5 returns 2 rows for "admin*", or LIKE-fallback returns matches for "admin*" literally.
        assert!(!m.is_empty());
    }
}
