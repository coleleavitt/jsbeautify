//! URL monitoring + baseline diffing + Discord webhook notifications.
//!
//! Ports jxscout's `crates/core/monitoring/` subsystem. Architecture:
//!
//! ```text
//! scheduler (cron loop)
//!   → fetch URL via reqwest
//!   → compute content_hash (SHA-256)
//!   → compare with stored baseline
//!   → if changed: record update, re-run analyzers, fire Discord webhook
//! ```
//!
//! This module provides the runtime engine and notification dispatch. The DB
//! operations delegate to [`crate::db::monitors`] and [`crate::db::files`].

use tokio::sync::broadcast;

use crate::db::{ProjectDb, monitors as db_monitors};

/// Event fired when monitored content changes.
#[derive(Debug, Clone)]
pub struct MonitoringEvent {
    pub monitor_id: i64,
    pub url: String,
    pub old_hash: Option<String>,
    pub new_hash: String,
    pub body: Vec<u8>,
}

/// Configuration for the monitoring system.
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    /// Discord webhook URL. If `None`, notifications are skipped.
    pub discord_webhook_url: Option<String>,
    /// Custom headers to include on every monitoring fetch.
    pub custom_headers: Vec<(String, String)>,
    /// Maximum body size to store (bytes).
    pub max_body_bytes: usize,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            discord_webhook_url: None,
            custom_headers: Vec::new(),
            max_body_bytes: 16 * 1024 * 1024,
        }
    }
}

/// The monitoring runtime. Spawn via [`MonitoringService::start`].
pub struct MonitoringService {
    db: ProjectDb,
    config: MonitoringConfig,
    client: reqwest::Client,
    /// Channel for subscribers to receive change events.
    pub events_tx: broadcast::Sender<MonitoringEvent>,
}

impl MonitoringService {
    /// Create a new monitoring service.
    #[must_use]
    pub fn new(db: ProjectDb, config: MonitoringConfig) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self {
            db,
            config,
            client: reqwest::Client::new(),
            events_tx,
        }
    }

    /// Run a single check cycle for all enabled monitors. Intended to be called
    /// from a cron/scheduler loop or manually from CLI.
    ///
    /// # Errors
    /// DB or network errors.
    pub async fn run_cycle(&self) -> Result<Vec<MonitoringEvent>, String> {
        let monitors = db_monitors::list(&self.db).map_err(|e| e.to_string())?;
        let mut events = Vec::new();

        for monitor in monitors.iter().filter(|m| m.enabled) {
            let urls = db_monitors::urls_for(&self.db, monitor.id).map_err(|e| e.to_string())?;
            for (url_pattern, _) in &urls {
                match self.check_url(monitor.id, url_pattern).await {
                    Ok(Some(ev)) => {
                        let _ = self.events_tx.send(ev.clone());
                        events.push(ev);
                    }
                    Ok(None) => {} // unchanged
                    Err(e) => {
                        db_monitors::record_failure(&self.db, monitor.id, url_pattern, &e)
                            .map_err(|e2| e2.to_string())?;
                    }
                }
            }
            db_monitors::record_run(&self.db, monitor.id).map_err(|e| e.to_string())?;
        }

        // Fire Discord notifications for all events.
        if let Some(ref webhook_url) = self.config.discord_webhook_url {
            for ev in &events {
                let _ = self.send_discord(webhook_url, ev).await;
            }
        }

        Ok(events)
    }

    async fn check_url(&self, monitor_id: i64, url: &str) -> Result<Option<MonitoringEvent>, String> {
        let mut req = self.client.get(url);
        for (k, v) in &self.config.custom_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let body = resp.bytes().await.map_err(|e| e.to_string())?;
        if body.len() > self.config.max_body_bytes {
            return Err(format!("body too large: {} bytes", body.len()));
        }
        let new_hash = ProjectDb::content_hash(&body);

        // Compare with stored baseline.
        let old_hash = crate::db::files::current_js_file(&self.db, url)
            .ok()
            .flatten()
            .map(|f| f.content_hash);

        if old_hash.as_deref() == Some(&new_hash) {
            return Ok(None); // unchanged
        }

        Ok(Some(MonitoringEvent {
            monitor_id,
            url: url.to_string(),
            old_hash,
            new_hash,
            body: body.to_vec(),
        }))
    }

    async fn send_discord(&self, webhook_url: &str, ev: &MonitoringEvent) -> Result<(), String> {
        let title = if ev.old_hash.is_none() {
            "New JS File Discovered"
        } else {
            "JS File Updated"
        };
        let payload = serde_json::json!({
            "embeds": [{
                "title": title,
                "description": format!("URL: {}\nNew hash: {}", ev.url, ev.new_hash),
                "color": 0x38_BD_F8,
            }]
        });
        let _ = self
            .client
            .post(webhook_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitoring_service_constructs() {
        let db = ProjectDb::open_in_memory().unwrap();
        let svc = MonitoringService::new(db, MonitoringConfig::default());
        assert!(svc.config.discord_webhook_url.is_none());
    }
}
