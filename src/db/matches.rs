//! `matches` + `analysis_run` repositories.
//!
//! Persists [`AnalyzerMatch`] rows produced by `analyzers::run_all` and tracks
//! seen/uninteresting state for triage.

use rusqlite::{Result, params};
use std::fmt::Write as _;

use super::ProjectDb;
use crate::analyzers::AnalyzerMatch;

/// Insert an `analysis_run` row and return its id.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn start_run(db: &ProjectDb, analyzer: &str, file_type: &str, file_id: i64) -> Result<i64> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO analysis_run (analyzer_name, file_type, file_id, started_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            params![analyzer, file_type, file_id],
        )?;
        Ok(c.last_insert_rowid())
    })
}

/// Mark an `analysis_run` complete.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn finish_run(db: &ProjectDb, run_id: i64) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE analysis_run SET finished_at = datetime('now') WHERE id = ?1",
            params![run_id],
        )?;
        Ok(())
    })
}

/// Persist a batch of matches under `run_id`.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn insert_matches(db: &ProjectDb, run_id: i64, matches: &[AnalyzerMatch]) -> Result<usize> {
    db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO matches
                 (analysis_run_id, match_kind, match_value, severity, provenance,
                  start_line, start_col, end_line, end_col, metadata, seen, is_uninteresting)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0)",
            )?;
            for m in matches {
                let provenance = serde_json::to_string(&m.provenance)
                    .ok()
                    .map_or_else(|| "ast".to_string(), |s| s.trim_matches('"').to_string());
                let metadata_str = m.metadata.as_ref().map(std::string::ToString::to_string);
                stmt.execute(params![
                    run_id,
                    m.match_kind,
                    m.match_value,
                    m.severity.as_str(),
                    provenance,
                    m.position.start.line,
                    m.position.start.column,
                    m.position.end.line,
                    m.position.end.column,
                    metadata_str,
                ])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    })
}

/// Filter knobs for [`get_matches`].
#[derive(Debug, Default, Clone)]
pub struct MatchFilter {
    pub kind: Option<String>,
    pub file_path_includes: Option<String>,
    pub value_includes: Option<String>,
    pub seen: Option<bool>,
    pub uninteresting: Option<bool>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredMatch {
    pub id: i64,
    pub match_kind: String,
    pub match_value: String,
    pub severity: String,
    pub provenance: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub seen: bool,
    pub is_uninteresting: bool,
    pub file_type: String,
    pub file_id: i64,
}

/// Query stored matches with optional filters.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn get_matches(db: &ProjectDb, filter: &MatchFilter) -> Result<Vec<StoredMatch>> {
    let mut sql = String::from(
        "SELECT m.id, m.match_kind, m.match_value, m.severity, m.provenance, \
         m.start_line, m.start_col, m.end_line, m.end_col, m.seen, m.is_uninteresting, \
         ar.file_type, ar.file_id \
         FROM matches m JOIN analysis_run ar ON m.analysis_run_id = ar.id WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(k) = &filter.kind {
        sql.push_str(" AND m.match_kind = ?");
        args.push(Box::new(k.clone()));
    }
    if let Some(v) = &filter.value_includes {
        sql.push_str(" AND m.match_value LIKE ?");
        args.push(Box::new(format!("%{v}%")));
    }
    if let Some(seen) = filter.seen {
        sql.push_str(" AND m.seen = ?");
        args.push(Box::new(i64::from(seen)));
    }
    if let Some(u) = filter.uninteresting {
        sql.push_str(" AND m.is_uninteresting = ?");
        args.push(Box::new(i64::from(u)));
    } else {
        // Default: hide uninteresting (matches jxscout's default behaviour).
        sql.push_str(" AND m.is_uninteresting = 0");
    }
    sql.push_str(" ORDER BY m.id DESC");
    if let Some(l) = filter.limit {
        write!(sql, " LIMIT {l}").ok();
    }
    if let Some(o) = filter.offset {
        write!(sql, " OFFSET {o}").ok();
    }

    db.with_conn(|c| {
        let mut stmt = c.prepare(&sql)?;
        let params_dyn: Vec<&dyn rusqlite::ToSql> = args.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(params_dyn.iter().copied()), |r| {
            Ok(StoredMatch {
                id: r.get(0)?,
                match_kind: r.get(1)?,
                match_value: r.get(2)?,
                severity: r.get(3)?,
                provenance: r.get(4)?,
                start_line: r.get::<_, i64>(5)? as u32,
                start_col: r.get::<_, i64>(6)? as u32,
                end_line: r.get::<_, i64>(7)? as u32,
                end_col: r.get::<_, i64>(8)? as u32,
                seen: r.get::<_, i64>(9)? != 0,
                is_uninteresting: r.get::<_, i64>(10)? != 0,
                file_type: r.get(11)?,
                file_id: r.get(12)?,
            })
        })?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        // Apply file_path filter post-load (requires joining files table by id+type).
        if let Some(needle) = &filter.file_path_includes {
            out.retain(|m| {
                let path = match m.file_type.as_str() {
                    "html" | "html_file" => fs_path_for(c, "html_files", m.file_id),
                    _ => fs_path_for(c, "js_files", m.file_id),
                };
                path.is_some_and(|p| p.contains(needle))
            });
        }
        Ok(out)
    })
}

fn fs_path_for(c: &rusqlite::Connection, table: &str, id: i64) -> Option<String> {
    c.query_row(
        &format!("SELECT fs_path FROM {table} WHERE id = ?1"),
        params![id],
        |r| r.get(0),
    )
    .ok()
}

/// Bulk-update `seen` on matches matching a filter.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn mark_seen(db: &ProjectDb, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("UPDATE matches SET seen = 1 WHERE id IN ({placeholders})");
    db.with_conn(|c| {
        let p: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        let n = c.execute(&sql, rusqlite::params_from_iter(p.iter().copied()))?;
        Ok(n)
    })
}

/// Mark all matches of a given kind as seen.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn mark_seen_by_kind(db: &ProjectDb, kind: &str) -> Result<usize> {
    db.with_conn(|c| c.execute("UPDATE matches SET seen = 1 WHERE match_kind = ?1", params![kind]))
}

/// Bulk-update `is_uninteresting`.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn mark_uninteresting(db: &ProjectDb, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!("UPDATE matches SET is_uninteresting = 1 WHERE id IN ({placeholders})");
    db.with_conn(|c| {
        let p: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
        c.execute(&sql, rusqlite::params_from_iter(p.iter().copied()))
    })
}

/// List the distinct match kinds that exist in the DB.
///
/// # Errors
/// Underlying rusqlite errors.
pub fn list_match_kinds(db: &ProjectDb) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT match_kind FROM matches ORDER BY match_kind")?;
        let rows = stmt.query_map(params![], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzers::match_kind::{FileType, PositionRange, Provenance};
    use crate::analyzers::regex_bank::Severity;

    fn mk(kind: &str, value: &str) -> AnalyzerMatch {
        AnalyzerMatch {
            match_kind: kind.into(),
            match_value: value.into(),
            position: PositionRange::new(1, 0, 1, value.len() as u32),
            file_type: FileType::Js,
            file_path: Some("/tmp/x.js".into()),
            severity: Severity::Info,
            provenance: Provenance::Ast,
            metadata: None,
        }
    }

    #[test]
    fn round_trip_matches() {
        let db = ProjectDb::open_in_memory().unwrap();
        let run = start_run(&db, "test", "js", 0).unwrap();
        let count = insert_matches(&db, run, &[mk("path", "/api/users"), mk("path", "/api/admin")]).unwrap();
        assert_eq!(count, 2);

        let kinds = list_match_kinds(&db).unwrap();
        assert_eq!(kinds, vec!["path".to_string()]);

        let all = get_matches(&db, &MatchFilter::default()).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn seen_and_uninteresting_filtering() {
        let db = ProjectDb::open_in_memory().unwrap();
        let run = start_run(&db, "test", "js", 0).unwrap();
        insert_matches(&db, run, &[mk("path", "/a"), mk("path", "/b")]).unwrap();

        let all = get_matches(&db, &MatchFilter::default()).unwrap();
        let id1 = all[0].id;
        mark_seen(&db, &[id1]).unwrap();

        let unseen = get_matches(
            &db,
            &MatchFilter {
                seen: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(unseen.len(), 1);

        mark_uninteresting(&db, &[id1]).unwrap();
        // Default filter hides uninteresting
        let visible = get_matches(&db, &MatchFilter::default()).unwrap();
        assert_eq!(visible.len(), 1);
        let with_uninteresting = get_matches(
            &db,
            &MatchFilter {
                uninteresting: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(with_uninteresting.len(), 1);
    }
}
