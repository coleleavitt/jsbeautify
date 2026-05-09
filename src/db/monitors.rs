//! `monitors` + `monitor_urls` + `monitoring_failed_urls` repositories.

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};

use super::ProjectDb;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub cron_expression: String,
    pub file_types: Option<String>,
    pub source_type: String,
    pub source_script: Option<String>,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CreateMonitor<'a> {
    pub name: &'a str,
    pub cron_expression: &'a str,
    pub file_types: Option<&'a str>,
    pub source_type: &'a str,
    pub source_script: Option<&'a str>,
}

/// # Errors
pub fn create(db: &ProjectDb, m: &CreateMonitor<'_>, urls: &[(&str, bool)]) -> Result<i64> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO monitors (name, cron_expression, file_types, source_type, source_script) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![m.name, m.cron_expression, m.file_types, m.source_type, m.source_script],
        )?;
        let id = c.last_insert_rowid();
        for (url, wild) in urls {
            c.execute(
                "INSERT OR IGNORE INTO monitor_urls (monitor_id, url_pattern, is_wildcard) VALUES (?1, ?2, ?3)",
                params![id, url, i64::from(*wild)],
            )?;
        }
        Ok(id)
    })
}

/// # Errors
pub fn list(db: &ProjectDb) -> Result<Vec<Monitor>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, name, cron_expression, file_types, source_type, source_script, enabled, \
             last_run_at, next_run_at FROM monitors ORDER BY id",
        )?;
        let rows = stmt.query_map(params![], |r| {
            Ok(Monitor {
                id: r.get(0)?,
                name: r.get(1)?,
                cron_expression: r.get(2)?,
                file_types: r.get(3)?,
                source_type: r.get(4)?,
                source_script: r.get(5)?,
                enabled: r.get::<_, i64>(6)? != 0,
                last_run_at: r.get(7)?,
                next_run_at: r.get(8)?,
            })
        })?;
        Ok(rows.flatten().collect())
    })
}

/// # Errors
pub fn urls_for(db: &ProjectDb, monitor_id: i64) -> Result<Vec<(String, bool)>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT url_pattern, is_wildcard FROM monitor_urls WHERE monitor_id = ?1")?;
        let rows = stmt.query_map(params![monitor_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
        })?;
        Ok(rows.flatten().collect())
    })
}

/// # Errors
pub fn update_cron(db: &ProjectDb, id: i64, cron: &str) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE monitors SET cron_expression = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![cron, id],
        )?)
    })
}

/// # Errors
pub fn record_run(db: &ProjectDb, id: i64) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE monitors SET last_run_at = datetime('now') WHERE id = ?1",
            params![id],
        )?)
    })
}

/// # Errors
pub fn record_failure(db: &ProjectDb, monitor_id: i64, url: &str, error: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO monitoring_failed_urls (monitor_id, url, error) VALUES (?1, ?2, ?3)",
            params![monitor_id, url, error],
        )?;
        Ok(())
    })
}

/// # Errors
pub fn delete(db: &ProjectDb, id: i64) -> Result<usize> {
    db.with_conn(|c| Ok(c.execute("DELETE FROM monitors WHERE id = ?1", params![id])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_lifecycle() {
        let db = ProjectDb::open_in_memory().unwrap();
        let id = create(
            &db,
            &CreateMonitor {
                name: "demo",
                cron_expression: "*/5 * * * *",
                file_types: Some(r#"["js"]"#),
                source_type: "url",
                source_script: None,
            },
            &[("https://target.com/*", true)],
        )
        .unwrap();
        let monitors = list(&db).unwrap();
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].id, id);

        let urls = urls_for(&db, id).unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].1);

        update_cron(&db, id, "0 * * * *").unwrap();
        record_run(&db, id).unwrap();
        record_failure(&db, id, "https://x", "timeout").unwrap();

        delete(&db, id).unwrap();
        assert!(list(&db).unwrap().is_empty());
    }
}
