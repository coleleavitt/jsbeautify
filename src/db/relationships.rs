//! `js_loader` / `iframe_loader` / `inline_js` / lazy chunk relationship persistence.
//!
//! Mirrors the four edge tables in `REPORT_06` §7.

use rusqlite::{Result, params};

use super::ProjectDb;

/// # Errors
pub fn record_js_loader(
    db: &ProjectDb,
    page_url: &str,
    page_file_path: &str,
    js_url: &str,
    js_file_path: &str,
) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO js_loader_relationships (page_url, page_file_path, js_url, js_file_path)
             VALUES (?1, ?2, ?3, ?4)",
            params![page_url, page_file_path, js_url, js_file_path],
        )?;
        Ok(())
    })
}

/// # Errors
pub fn record_iframe_loader(
    db: &ProjectDb,
    page_url: &str,
    page_file_path: &str,
    iframe_url: &str,
    iframe_file_path: &str,
) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO iframe_loader_relationships
             (page_url, page_file_path, iframe_url, iframe_file_path)
             VALUES (?1, ?2, ?3, ?4)",
            params![page_url, page_file_path, iframe_url, iframe_file_path],
        )?;
        Ok(())
    })
}

/// # Errors
pub fn record_inline_js(db: &ProjectDb, html_file_id: i64, js_file_id: i64, block_index: i64, line: u32) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO html_files_inline_js (html_file_id, js_file_id, block_index, line)
             VALUES (?1, ?2, ?3, ?4)",
            params![html_file_id, js_file_id, block_index, line],
        )?;
        Ok(())
    })
}

/// # Errors
pub fn record_lazy_chunk(db: &ProjectDb, parent_js_id: i64, chunk_url: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO discovered_lazy_loaded_js_files (parent_js_id, chunk_url) VALUES (?1, ?2)",
            params![parent_js_id, chunk_url],
        )?;
        Ok(())
    })
}

/// All JS URLs loaded by an HTML page (jxscout's `get-loaded-js-files` SQL).
///
/// # Errors
pub fn js_files_loaded_by(db: &ProjectDb, page_url: &str) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT js_url FROM js_loader_relationships WHERE page_url = ?1")?;
        let rows = stmt.query_map(params![page_url], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    })
}

/// All HTML pages that load `js_url`. (`get-js-file-loader-page`)
///
/// # Errors
pub fn pages_loading_js(db: &ProjectDb, js_url: &str) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT page_url FROM js_loader_relationships WHERE js_url = ?1")?;
        let rows = stmt.query_map(params![js_url], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    })
}

/// All iframe URLs embedded by a page. (`get-loaded-iframes`)
///
/// # Errors
pub fn iframes_loaded_by(db: &ProjectDb, page_url: &str) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT iframe_url FROM iframe_loader_relationships WHERE page_url = ?1")?;
        let rows = stmt.query_map(params![page_url], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let db = ProjectDb::open_in_memory().unwrap();
        record_js_loader(&db, "https://t/p1", "/p1.html", "https://t/a.js", "/a.js").unwrap();
        record_js_loader(&db, "https://t/p1", "/p1.html", "https://t/b.js", "/b.js").unwrap();
        record_iframe_loader(&db, "https://t/p1", "/p1.html", "https://embed/x", "/x.html").unwrap();

        let js = js_files_loaded_by(&db, "https://t/p1").unwrap();
        assert_eq!(js.len(), 2);

        let pages = pages_loading_js(&db, "https://t/a.js").unwrap();
        assert_eq!(pages, vec!["https://t/p1".to_string()]);

        let iframes = iframes_loaded_by(&db, "https://t/p1").unwrap();
        assert_eq!(iframes, vec!["https://embed/x".to_string()]);
    }
}
