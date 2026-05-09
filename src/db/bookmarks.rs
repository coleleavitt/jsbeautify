//! Bookmark + bookmark group CRUD.
//!
//! Mirrors jxscout's `crates/core/project-db/src/bookmarks.rs`.

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};

use super::ProjectDb;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkGroup {
    pub id: i64,
    pub name: String,
    pub highlight_color: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: i64,
    pub group_id: i64,
    pub group_name: Option<String>,
    pub file_path: String,
    pub file_type: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub note: Option<String>,
    pub match_id: Option<i64>,
    pub created_at: String,
}

/// # Errors
pub fn create_group(db: &ProjectDb, name: &str, highlight_color: Option<&str>) -> Result<BookmarkGroup> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO bookmark_groups (name, highlight_color) VALUES (?1, ?2)",
            params![name, highlight_color],
        )?;
        let id = c.last_insert_rowid();
        Ok(BookmarkGroup {
            id,
            name: name.to_string(),
            highlight_color: highlight_color.map(String::from),
            created_at: "now".into(),
        })
    })
}

/// # Errors
pub fn list_groups(db: &ProjectDb) -> Result<Vec<BookmarkGroup>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT id, name, highlight_color, created_at FROM bookmark_groups ORDER BY name")?;
        let rows = stmt.query_map(params![], |r| {
            Ok(BookmarkGroup {
                id: r.get(0)?,
                name: r.get(1)?,
                highlight_color: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?;
        Ok(rows.flatten().collect())
    })
}

/// # Errors
pub fn delete_group(db: &ProjectDb, name: &str) -> Result<usize> {
    db.with_conn(|c| Ok(c.execute("DELETE FROM bookmark_groups WHERE name = ?1", params![name])?))
}

/// # Errors
pub fn update_group(
    db: &ProjectDb,
    name: &str,
    new_name: Option<&str>,
    highlight_color: Option<&str>,
) -> Result<usize> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE bookmark_groups SET
              name = COALESCE(?1, name),
              highlight_color = COALESCE(?2, highlight_color)
             WHERE name = ?3",
            params![new_name, highlight_color, name],
        )?;
        Ok(n)
    })
}

#[derive(Debug, Clone)]
pub struct CreateBookmark<'a> {
    pub group_name: &'a str,
    pub file_path: &'a str,
    pub file_type: &'a str,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub note: Option<&'a str>,
    pub match_id: Option<i64>,
}

/// # Errors
pub fn create_bookmark(db: &ProjectDb, b: &CreateBookmark<'_>) -> Result<i64> {
    db.with_conn(|c| {
        let group_id: i64 = c.query_row(
            "SELECT id FROM bookmark_groups WHERE name = ?1",
            params![b.group_name],
            |r| r.get(0),
        )?;
        c.execute(
            "INSERT INTO bookmarks
             (group_id, file_path, file_type, start_line, start_col, end_line, end_col, note, match_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                group_id,
                b.file_path,
                b.file_type,
                b.start_line,
                b.start_col,
                b.end_line,
                b.end_col,
                b.note,
                b.match_id,
            ],
        )?;
        Ok(c.last_insert_rowid())
    })
}

/// # Errors
pub fn list_bookmarks(db: &ProjectDb, group: Option<&str>, file_path: Option<&str>) -> Result<Vec<Bookmark>> {
    let mut sql = String::from(
        "SELECT b.id, b.group_id, g.name, b.file_path, b.file_type, b.start_line, b.start_col, \
         b.end_line, b.end_col, b.note, b.match_id, b.created_at \
         FROM bookmarks b JOIN bookmark_groups g ON b.group_id = g.id WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(name) = group {
        sql.push_str(" AND g.name = ?");
        args.push(Box::new(name.to_string()));
    }
    if let Some(p) = file_path {
        sql.push_str(" AND b.file_path = ?");
        args.push(Box::new(p.to_string()));
    }
    sql.push_str(" ORDER BY b.id DESC");

    db.with_conn(|c| {
        let mut stmt = c.prepare(&sql)?;
        let p_dyn: Vec<&dyn rusqlite::ToSql> = args.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(p_dyn.iter().copied()), |r| {
            Ok(Bookmark {
                id: r.get(0)?,
                group_id: r.get(1)?,
                group_name: r.get(2)?,
                file_path: r.get(3)?,
                file_type: r.get(4)?,
                start_line: r.get::<_, i64>(5)? as u32,
                start_col: r.get::<_, i64>(6)? as u32,
                end_line: r.get::<_, i64>(7)? as u32,
                end_col: r.get::<_, i64>(8)? as u32,
                note: r.get(9)?,
                match_id: r.get(10)?,
                created_at: r.get(11)?,
            })
        })?;
        Ok(rows.flatten().collect())
    })
}

/// # Errors
pub fn update_bookmark(
    db: &ProjectDb,
    id: i64,
    new_group: Option<&str>,
    note: Option<&str>,
    new_file_path: Option<&str>,
) -> Result<usize> {
    db.with_conn(|c| {
        let group_id: Option<i64> = match new_group {
            Some(name) => c
                .query_row("SELECT id FROM bookmark_groups WHERE name = ?1", params![name], |r| {
                    r.get(0)
                })
                .ok(),
            None => None,
        };
        let n = c.execute(
            "UPDATE bookmarks SET
              group_id  = COALESCE(?1, group_id),
              note      = COALESCE(?2, note),
              file_path = COALESCE(?3, file_path)
             WHERE id = ?4",
            params![group_id, note, new_file_path, id],
        )?;
        Ok(n)
    })
}

/// # Errors
pub fn delete_bookmark(db: &ProjectDb, id: i64) -> Result<usize> {
    db.with_conn(|c| Ok(c.execute("DELETE FROM bookmarks WHERE id = ?1", params![id])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_crud() {
        let db = ProjectDb::open_in_memory().unwrap();
        let g = create_group(&db, "XSS", Some("rgba(255,0,0,0.1)")).unwrap();
        assert!(g.id > 0);
        assert_eq!(list_groups(&db).unwrap().len(), 1);
        update_group(&db, "XSS", Some("XSS Sinks"), None).unwrap();
        let groups = list_groups(&db).unwrap();
        assert_eq!(groups[0].name, "XSS Sinks");
        delete_group(&db, "XSS Sinks").unwrap();
        assert!(list_groups(&db).unwrap().is_empty());
    }

    #[test]
    fn bookmark_crud() {
        let db = ProjectDb::open_in_memory().unwrap();
        create_group(&db, "default", None).unwrap();
        let id = create_bookmark(
            &db,
            &CreateBookmark {
                group_name: "default",
                file_path: "/tmp/x.js",
                file_type: "js",
                start_line: 10,
                start_col: 0,
                end_line: 20,
                end_col: 5,
                note: Some("interesting"),
                match_id: None,
            },
        )
        .unwrap();
        let all = list_bookmarks(&db, None, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].note.as_deref(), Some("interesting"));
        update_bookmark(&db, id, None, Some("very interesting"), None).unwrap();
        let all = list_bookmarks(&db, None, None).unwrap();
        assert_eq!(all[0].note.as_deref(), Some("very interesting"));
        delete_bookmark(&db, id).unwrap();
        assert!(list_bookmarks(&db, None, None).unwrap().is_empty());
    }
}
