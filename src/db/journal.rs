//! `journal_entries` repository — test/hypothesis tracker.

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

use super::ProjectDb;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    Test,
    Hypothesis,
    Observation,
    Recon,
}

impl EntryType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Hypothesis => "hypothesis",
            Self::Observation => "observation",
            Self::Recon => "recon",
        }
    }
}

impl std::str::FromStr for EntryType {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "test" => Ok(Self::Test),
            "hypothesis" => Ok(Self::Hypothesis),
            "observation" => Ok(Self::Observation),
            "recon" => Ok(Self::Recon),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Untested,
    Confirmed,
    Disproved,
}

impl Status {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Untested => "untested",
            Self::Confirmed => "confirmed",
            Self::Disproved => "disproved",
        }
    }
}

impl std::str::FromStr for Status {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "untested" => Ok(Self::Untested),
            "confirmed" => Ok(Self::Confirmed),
            "disproved" => Ok(Self::Disproved),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: i64,
    pub entry_type: String,
    pub target: String,
    pub description: String,
    pub tags: Option<String>,
    pub status: String,
    pub evidence: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct CreateEntry<'a> {
    pub entry_type: EntryType,
    pub target: &'a str,
    pub description: &'a str,
    pub tags: Option<&'a str>,
    pub status: Status,
    pub evidence: Option<&'a str>,
}

/// # Errors
pub fn add(db: &ProjectDb, e: &CreateEntry<'_>) -> Result<i64> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO journal_entries (entry_type, target, description, tags, status, evidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                e.entry_type.as_str(),
                e.target,
                e.description,
                e.tags,
                e.status.as_str(),
                e.evidence
            ],
        )?;
        Ok(c.last_insert_rowid())
    })
}

#[derive(Debug, Default, Clone)]
pub struct JournalFilter<'a> {
    pub target_substring: Option<&'a str>,
    pub entry_type: Option<EntryType>,
    pub status: Option<Status>,
    pub tags_substring: Option<&'a str>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// # Errors
pub fn list(db: &ProjectDb, f: &JournalFilter<'_>) -> Result<Vec<JournalEntry>> {
    let mut sql = String::from(
        "SELECT id, entry_type, target, description, tags, status, evidence, created_at, updated_at \
         FROM journal_entries WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(s) = f.target_substring {
        sql.push_str(" AND target LIKE ?");
        args.push(Box::new(format!("%{s}%")));
    }
    if let Some(t) = f.entry_type {
        sql.push_str(" AND entry_type = ?");
        args.push(Box::new(t.as_str().to_string()));
    }
    if let Some(s) = f.status {
        sql.push_str(" AND status = ?");
        args.push(Box::new(s.as_str().to_string()));
    }
    if let Some(t) = f.tags_substring {
        sql.push_str(" AND tags LIKE ?");
        args.push(Box::new(format!("%{t}%")));
    }
    sql.push_str(" ORDER BY id DESC");
    if let Some(l) = f.limit {
        write!(sql, " LIMIT {l}").ok();
    }
    if let Some(o) = f.offset {
        write!(sql, " OFFSET {o}").ok();
    }

    db.with_conn(|c| {
        let mut stmt = c.prepare(&sql)?;
        let p_dyn: Vec<&dyn rusqlite::ToSql> = args.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(p_dyn.iter().copied()), |r| {
            Ok(JournalEntry {
                id: r.get(0)?,
                entry_type: r.get(1)?,
                target: r.get(2)?,
                description: r.get(3)?,
                tags: r.get(4)?,
                status: r.get(5)?,
                evidence: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
        Ok(rows.flatten().collect())
    })
}

/// Aggregate counts by `(entry_type, status)`.
///
/// # Errors
pub fn summary(db: &ProjectDb) -> Result<Vec<(String, String, i64)>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT entry_type, status, COUNT(*) FROM journal_entries
             GROUP BY entry_type, status ORDER BY entry_type, status",
        )?;
        let rows = stmt.query_map(params![], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?;
        Ok(rows.flatten().collect())
    })
}

/// # Errors
pub fn delete(db: &ProjectDb, id: i64) -> Result<usize> {
    db.with_conn(|c| c.execute("DELETE FROM journal_entries WHERE id = ?1", params![id]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_summary() {
        let db = ProjectDb::open_in_memory().unwrap();
        add(
            &db,
            &CreateEntry {
                entry_type: EntryType::Hypothesis,
                target: "/api/v2/users",
                description: "IDOR — endpoint may not check user id",
                tags: Some("idor,authorization"),
                status: Status::Untested,
                evidence: None,
            },
        )
        .unwrap();
        add(
            &db,
            &CreateEntry {
                entry_type: EntryType::Test,
                target: "/api/v2/admin",
                description: "tested admin endpoint with low-priv user",
                tags: Some("idor"),
                status: Status::Confirmed,
                evidence: Some("evidence/admin.txt"),
            },
        )
        .unwrap();

        let all = list(&db, &JournalFilter::default()).unwrap();
        assert_eq!(all.len(), 2);

        let confirmed = list(
            &db,
            &JournalFilter {
                status: Some(Status::Confirmed),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(confirmed.len(), 1);

        let s = summary(&db).unwrap();
        assert_eq!(s.len(), 2);
    }
}
