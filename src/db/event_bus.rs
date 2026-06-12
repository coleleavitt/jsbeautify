//! Persistent event bus backed by SQLite.
//!
//! Implements jxscout's job-queue pattern (REPORT_06 §10):
//!   1. Producer publishes an `events` row.
//!   2. For each registered subscriber on the topic, a `subscriber_jobs` row
//!      is inserted in `pending` state.
//!   3. Workers atomically claim jobs by updating `pending → running` with
//!      `locked_at = now()` and a heartbeat.
//!   4. On success: `running → done`. On failure: `running → failed` with
//!      `attempts++`. Stale `running` rows (heartbeat older than threshold)
//!      can be requeued by a sweeper.

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};

use super::ProjectDb;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub topic: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscriber {
    pub id: i64,
    pub topic: String,
    pub queue_name: String,
    pub concurrency: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub subscriber: String,
    pub event_id: i64,
    pub status: String,
    pub attempts: i64,
}

/// Register a subscriber. Idempotent on `queue_name`.
///
/// # Errors
pub fn register_subscriber(db: &ProjectDb, topic: &str, queue_name: &str, concurrency: u32) -> Result<i64> {
    db.with_conn(|c| {
        c.execute(
            "INSERT OR IGNORE INTO subscribers (topic, queue_name, concurrency) VALUES (?1, ?2, ?3)",
            params![topic, queue_name, concurrency as i64],
        )?;
        let id: i64 = c.query_row(
            "SELECT id FROM subscribers WHERE queue_name = ?1",
            params![queue_name],
            |r| r.get(0),
        )?;
        Ok(id)
    })
}

/// Publish an event. For every registered subscriber on the topic, insert a
/// `subscriber_jobs` row in `pending` state.
///
/// # Errors
pub fn publish(db: &ProjectDb, topic: &str, payload: &serde_json::Value) -> Result<i64> {
    db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO events (topic, payload) VALUES (?1, ?2)",
            params![topic, payload.to_string()],
        )?;
        let event_id = tx.last_insert_rowid();

        let mut stmt = tx.prepare("SELECT queue_name FROM subscribers WHERE topic = ?1")?;
        let subs: Vec<String> = stmt
            .query_map(params![topic], |r| r.get::<_, String>(0))?
            .flatten()
            .collect();
        drop(stmt);
        for queue in subs {
            tx.execute(
                "INSERT INTO subscriber_jobs (subscriber, event_id, status) VALUES (?1, ?2, 'pending')",
                params![queue, event_id],
            )?;
        }
        tx.commit()?;
        Ok(event_id)
    })
}

/// Atomically claim one pending job for a subscriber.
///
/// # Errors
pub fn claim(db: &ProjectDb, subscriber: &str) -> Result<Option<(Job, Event)>> {
    db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        let row = tx
            .query_row(
                "SELECT j.id, j.subscriber, j.event_id, j.status, j.attempts,
                        e.id, e.topic, e.payload, e.created_at
                 FROM subscriber_jobs j JOIN events e ON e.id = j.event_id
                 WHERE j.subscriber = ?1 AND j.status = 'pending'
                   AND j.run_at <= datetime('now')
                 ORDER BY j.id LIMIT 1",
                params![subscriber],
                |r| {
                    Ok((
                        Job {
                            id: r.get(0)?,
                            subscriber: r.get(1)?,
                            event_id: r.get(2)?,
                            status: r.get(3)?,
                            attempts: r.get(4)?,
                        },
                        Event {
                            id: r.get(5)?,
                            topic: r.get(6)?,
                            payload: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::Value::Null),
                            created_at: r.get(8)?,
                        },
                    ))
                },
            )
            .ok();
        let result = if let Some((mut job, ev)) = row {
            let updated = tx.execute(
                "UPDATE subscriber_jobs SET status = 'running', locked_at = datetime('now'),
                  heartbeat_at = datetime('now'), attempts = attempts + 1
                 WHERE id = ?1 AND status = 'pending'",
                params![job.id],
            )?;
            if updated == 1 {
                job.status = "running".into();
                job.attempts += 1;
                Some((job, ev))
            } else {
                None
            }
        } else {
            None
        };
        tx.commit()?;
        Ok(result)
    })
}

/// Mark a job done.
///
/// # Errors
pub fn complete(db: &ProjectDb, job_id: i64) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE subscriber_jobs SET status = 'done', locked_at = NULL WHERE id = ?1",
            params![job_id],
        )?;
        Ok(())
    })
}

/// Mark a job failed.
///
/// # Errors
pub fn fail(db: &ProjectDb, job_id: i64, error: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE subscriber_jobs SET status = 'failed', error = ?2, locked_at = NULL WHERE id = ?1",
            params![job_id, error],
        )?;
        Ok(())
    })
}

/// Heartbeat a running job to keep its lease.
///
/// # Errors
pub fn heartbeat(db: &ProjectDb, job_id: i64) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE subscriber_jobs SET heartbeat_at = datetime('now') WHERE id = ?1",
            params![job_id],
        )?;
        Ok(())
    })
}

/// Reclaim jobs whose heartbeat is older than `staleness_seconds`.
///
/// # Errors
pub fn requeue_stale(db: &ProjectDb, staleness_seconds: i64) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE subscriber_jobs SET status = 'pending', locked_at = NULL
             WHERE status = 'running' AND
                   (heartbeat_at IS NULL OR heartbeat_at < datetime('now', ?1))",
            params![format!("-{staleness_seconds} seconds")],
        )?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_claim_complete() {
        let db = ProjectDb::open_in_memory().unwrap();
        register_subscriber(&db, "js_file_saved", "analyzer", 2).unwrap();
        register_subscriber(&db, "js_file_saved", "wordlist", 1).unwrap();

        let _ev = publish(&db, "js_file_saved", &serde_json::json!({"url": "/x.js"})).unwrap();
        let _ev2 = publish(&db, "js_file_saved", &serde_json::json!({"url": "/y.js"})).unwrap();

        let (job, ev) = claim(&db, "analyzer")
            .expect("claim db error")
            .expect("expected a pending job for analyzer");
        assert_eq!(job.status, "running");
        assert_eq!(ev.topic, "js_file_saved");
        complete(&db, job.id).unwrap();

        // Wordlist still has both pending; analyzer has 1 left.
        let _ = claim(&db, "wordlist").unwrap().unwrap();
        let _ = claim(&db, "wordlist").unwrap().unwrap();
        assert!(claim(&db, "wordlist").unwrap().is_none());
    }

    #[test]
    fn fail_records_error_and_attempts() {
        let db = ProjectDb::open_in_memory().unwrap();
        register_subscriber(&db, "x", "a", 1).unwrap();
        publish(&db, "x", &serde_json::Value::Null).unwrap();
        let (job, _) = claim(&db, "a").unwrap().unwrap();
        fail(&db, job.id, "boom").unwrap();
        // Was it persisted?
        let attempts: i64 = db
            .with_conn(|c| {
                c.query_row(
                    "SELECT attempts FROM subscriber_jobs WHERE id = ?1",
                    params![job.id],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert_eq!(attempts, 1);
    }
}
