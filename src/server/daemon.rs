//! The `serve` daemon — boots and wires every previously-unwired subsystem
//! into one running process.
//!
//! Until now js-beautify-rs's jxscout-derived modules (proxy, WebSocket server,
//! project DB, hooks, monitoring, event bus) existed only as library code with
//! no binary entry point — `main()` was synchronous, file-in/file-out. This
//! module is the orchestrator that turns them into a live service:
//!
//! ```text
//!            ┌──────────────── Mindemon (Firefox panel) ───────────────┐
//!            │  POST /ingest, /ingest/batch                            │
//!            ▼                                                          │
//!   HTTP API :3333 ─┐                                                  │
//!                   ├─► Pipeline ─► ProjectDb (SQLite)                 │
//!   MITM proxy :8080┘      │           hooks / event_bus               │
//!   (PipelineIngestor)     └─► WsServer :9222 ──► editor clients ◄─────┘
//!   monitoring cron ───────────► Pipeline / WsServer
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::analyzers::AnalyzerConfig;
use crate::db::ProjectDb;
use crate::monitoring::{MonitoringConfig, MonitoringService};
use crate::pipeline::{Pipeline, PipelineConfig, PipelineIngestor};
use crate::proxy::{ProxyConfig, ProxyServer};
use crate::server::{ApiConfig, ApiServer};
use crate::websocket::{WsConfig, WsMessage, WsServer};

/// Daemon configuration (populated from CLI flags).
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub project_name: String,
    pub http_bind: SocketAddr,
    pub ws_bind: SocketAddr,
    pub proxy_bind: SocketAddr,
    pub intercept_tls: bool,
    pub scope: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub discord_webhook: Option<String>,
    /// Seconds between monitoring cycles (0 disables the monitor loop).
    pub monitor_interval_secs: u64,
    /// Run the deobfuscator on minified JS during ingestion.
    pub deobfuscate: bool,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            project_name: "default".to_string(),
            http_bind: "127.0.0.1:3333".parse().expect("addr"),
            ws_bind: "127.0.0.1:9222".parse().expect("addr"),
            proxy_bind: "127.0.0.1:8080".parse().expect("addr"),
            intercept_tls: false,
            scope: Vec::new(),
            out_of_scope: Vec::new(),
            discord_webhook: None,
            monitor_interval_secs: 0,
            deobfuscate: true,
        }
    }
}

impl ServeConfig {
    /// Project working directory: `~/.jsbeautify/projects/<name>/`.
    #[must_use]
    pub fn workdir(&self) -> PathBuf {
        crate::db::default_project_db_path(&self.project_name)
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Boot the daemon and run until shutdown (Ctrl-C). Spawns one task per
/// subsystem and waits.
///
/// # Errors
/// Returns an error if the project DB cannot be opened or a listener fails to
/// bind.
pub async fn serve(cfg: ServeConfig) -> Result<(), String> {
    let workdir = cfg.workdir();
    std::fs::create_dir_all(&workdir).map_err(|e| format!("create workdir {}: {e}", workdir.display()))?;

    // ── Project DB ───────────────────────────────────────────────────────────
    let db_path = crate::db::default_project_db_path(&cfg.project_name);
    let db = ProjectDb::open(&db_path).map_err(|e| format!("open project db: {e}"))?;
    db.upsert_project(&cfg.project_name, &workdir.to_string_lossy())
        .map_err(|e| format!("register project: {e}"))?;

    // ── WebSocket server (result stream to editors) ──────────────────────────
    let ws = Arc::new(WsServer::new(WsConfig { bind: cfg.ws_bind }));
    let ws_tx: broadcast::Sender<WsMessage> = ws.tx.clone();

    // ── Shared ingestion pipeline ────────────────────────────────────────────
    let pipeline = Arc::new(
        Pipeline::new(
            db.clone(),
            PipelineConfig {
                workdir: workdir.clone(),
                analyzer: AnalyzerConfig::default(),
                deobfuscate: cfg.deobfuscate,
                ..PipelineConfig::default()
            },
        )
        .with_ws(ws_tx.clone()),
    );

    // ── MITM proxy (feeds the same pipeline) ─────────────────────────────────
    let proxy_cfg = ProxyConfig {
        bind: cfg.proxy_bind,
        intercept_tls: cfg.intercept_tls,
        ca_dir: crate::proxy::ca::default_dir(),
        max_body_bytes: 16 * 1024 * 1024,
        in_scope: cfg.scope.clone(),
        out_of_scope: cfg.out_of_scope.clone(),
    };
    let proxy = ProxyServer::new(proxy_cfg, Arc::new(PipelineIngestor::new(pipeline.clone())));

    // CA PEM (if TLS interception is on) is served by the HTTP API at /ca.crt.
    let ca_pem = if cfg.intercept_tls {
        crate::proxy::ca::CertificateAuthority::load_or_generate(crate::proxy::ca::default_dir())
            .ok()
            .map(|ca| ca.cert_pem().to_string())
    } else {
        None
    };

    // ── HTTP ingestion API ───────────────────────────────────────────────────
    let api = ApiServer::new(
        ApiConfig {
            bind: cfg.http_bind,
            ca_pem,
            ..ApiConfig::default()
        },
        pipeline.clone(),
    );

    print_banner(&cfg, &workdir, &db_path);

    let tasks = spawn_subsystems(ws, api, proxy, db, ws_tx, &cfg);

    // ── Wait for shutdown ────────────────────────────────────────────────────
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\n[daemon] shutdown requested; stopping.");
        }
        _ = wait_all(tasks) => {
            eprintln!("[daemon] all subsystems exited.");
        }
    }
    Ok(())
}

/// Spawn one tokio task per subsystem (WebSocket, HTTP API, proxy, monitor).
fn spawn_subsystems(
    ws: Arc<WsServer>,
    api: ApiServer,
    proxy: ProxyServer,
    db: ProjectDb,
    ws_tx: broadcast::Sender<WsMessage>,
    cfg: &ServeConfig,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::new();
    tasks.push(tokio::spawn(async move {
        if let Err(e) = ws.serve().await {
            eprintln!("[ws] server stopped: {e}");
        }
    }));
    tasks.push(tokio::spawn(async move {
        if let Err(e) = api.serve().await {
            eprintln!("[api] server stopped: {e}");
        }
    }));
    tasks.push(tokio::spawn(async move {
        if let Err(e) = proxy.serve().await {
            eprintln!("[proxy] server stopped: {e}");
        }
    }));
    if cfg.monitor_interval_secs > 0 {
        tasks.push(spawn_monitor(db, ws_tx, cfg));
    }
    tasks
}

/// Spawn the monitoring cron loop: every `interval` seconds run a check cycle
/// and broadcast any change events to WebSocket clients.
fn spawn_monitor(db: ProjectDb, ws_tx: broadcast::Sender<WsMessage>, cfg: &ServeConfig) -> tokio::task::JoinHandle<()> {
    let interval = Duration::from_secs(cfg.monitor_interval_secs);
    let mon_cfg = MonitoringConfig {
        discord_webhook_url: cfg.discord_webhook.clone(),
        ..MonitoringConfig::default()
    };
    tokio::spawn(async move {
        let service = MonitoringService::new(db, mon_cfg);
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match service.run_cycle().await {
                Ok(events) => broadcast_monitor_changes(&ws_tx, events),
                Err(e) => eprintln!("[monitor] cycle error: {e}"),
            }
        }
    })
}

/// Broadcast monitoring change events to all connected WebSocket clients.
fn broadcast_monitor_changes(ws_tx: &broadcast::Sender<WsMessage>, events: Vec<crate::monitoring::MonitoringEvent>) {
    for ev in events {
        // A send error only means no editor is currently subscribed — fine.
        ws_tx
            .send(WsMessage::Broadcast(serde_json::json!({
                "kind": "monitor_change",
                "url": ev.url,
                "oldHash": ev.old_hash,
                "newHash": ev.new_hash,
            })))
            .ok();
    }
}

async fn wait_all(tasks: Vec<tokio::task::JoinHandle<()>>) {
    for t in tasks {
        if let Err(e) = t.await {
            eprintln!("[daemon] subsystem task join error: {e}");
        }
    }
}

fn print_banner(cfg: &ServeConfig, workdir: &std::path::Path, db_path: &std::path::Path) {
    eprintln!("┌─ js-beautify-rs daemon ─────────────────────────────────────");
    eprintln!("│ project    : {}", cfg.project_name);
    eprintln!("│ workdir    : {}", workdir.display());
    eprintln!("│ db         : {}", db_path.display());
    eprintln!("│ HTTP API   : http://{}  (POST /ingest, /ingest/batch)", cfg.http_bind);
    eprintln!("│ WebSocket  : ws://{}   (live results to editors)", cfg.ws_bind);
    eprintln!(
        "│ MITM proxy : http://{}  (intercept_tls={})",
        cfg.proxy_bind, cfg.intercept_tls
    );
    if cfg.intercept_tls {
        eprintln!(
            "│ CA trust   : GET http://{}/ca.crt then import into the browser",
            cfg.http_bind
        );
    }
    if !cfg.scope.is_empty() {
        eprintln!("│ scope      : {}", cfg.scope.join(", "));
    }
    if cfg.monitor_interval_secs > 0 {
        eprintln!("│ monitor    : every {}s", cfg.monitor_interval_secs);
    }
    eprintln!(
        "│ Mindemon   : set backend URL to http://{} and 'Send to Backend'",
        cfg.http_bind
    );
    eprintln!("└─────────────────────────────────────────────────────────────");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workdir_under_project() {
        let cfg = ServeConfig {
            project_name: "acme".into(),
            ..ServeConfig::default()
        };
        let wd = cfg.workdir();
        assert!(wd.to_string_lossy().contains("acme"));
    }

    #[tokio::test]
    async fn daemon_boots_and_serves_health() {
        // Bind all three subsystems on ephemeral ports.
        let pick = || {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let a = l.local_addr().unwrap();
            drop(l);
            a
        };
        let http = pick();
        let cfg = ServeConfig {
            project_name: format!("daemon-test-{}", std::process::id()),
            http_bind: http,
            ws_bind: pick(),
            proxy_bind: pick(),
            ..ServeConfig::default()
        };
        tokio::spawn(async move {
            let _ = serve(cfg).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let resp = reqwest::Client::new()
            .get(format!("http://{http}/health"))
            .send()
            .await
            .expect("health request");
        assert_eq!(resp.status(), 200);
        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(json["ok"], true);
    }
}
