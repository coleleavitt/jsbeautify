//! Unified ingestion pipeline.
//!
//! This is the seam between capture sources (Mindemon browser dumps over HTTP,
//! the MITM proxy, fs ingestion) and the analysis backend (beautify /
//! deobfuscate / analyzers / sourcemap reversal / DB / WS).
//!
//! This is the integration layer that turns js-beautify-rs's previously-unwired
//! library modules (`db`, `websocket`, `hooks`, `analyzers`, `sourcemap`,
//! `relationships`) into a running system. One [`Pipeline`] is shared (behind an
//! `Arc`) by the HTTP API server, the proxy ingestor, and the daemon.
//!
//! ## Flow for one asset
//!
//! ```text
//! ingest_asset(url, content_type, bytes, source)
//!   ├─ classify FileType (js / html / source-map)
//!   ├─ content_hash + history-aware upsert into js_files / html_files
//!   ├─ write raw bytes to <workdir>/raw/<path>
//!   ├─ JS: beautify (+ deobfuscate when minified) → <workdir>/beautified/<path>
//!   ├─ run analyzers::run_all → persist matches (analysis_run + matches rows)
//!   ├─ source-map: reverse sourcesContent → write + re-analyze each original
//!   ├─ html: build AssetGraph (js/iframe/lazy-chunk relationships)
//!   ├─ publish event_bus events + fire hooks
//!   └─ broadcast JsFilesUpdate / HtmlFilesUpdate / MatchesUpdate over WS
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::analyzers::{self, AnalyzerConfig, AnalyzerMatch, FileType};
use crate::db::files::IngestionSource;
use crate::db::{self, ProjectDb};
use crate::hooks::{self, HooksMap};
use crate::websocket::WsMessage;
use crate::{Options, beautify};

/// What kind of asset we were handed. Drives beautification + analyzer choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    JavaScript,
    Html,
    Css,
    Json,
    SourceMap,
    /// Binary / not analyzable (font, image, wasm, media). Stored raw only.
    Other,
}

impl AssetKind {
    /// Infer from a content-type and/or URL.
    #[must_use]
    pub fn classify(url: &str, content_type: &str) -> Self {
        let ct = content_type.to_ascii_lowercase();
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let extension = std::path::Path::new(path).extension().and_then(std::ffi::OsStr::to_str);

        if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("map")) {
            return Self::SourceMap;
        }
        if ct.contains("javascript") || ct.contains("ecmascript") {
            return Self::JavaScript;
        }
        if ct.contains("text/html") {
            return Self::Html;
        }
        if ct.contains("text/css") {
            return Self::Css;
        }
        if ct.contains("application/json") || ct.contains("+json") {
            return Self::Json;
        }
        // Fall back to extension.
        if extension.is_some_and(|ext| {
            ["mjs", "cjs", "js", "jsx", "ts", "tsx"]
                .iter()
                .any(|expected| ext.eq_ignore_ascii_case(expected))
        }) {
            Self::JavaScript
        } else if extension.is_some_and(|ext| {
            ["html", "htm"]
                .iter()
                .any(|expected| ext.eq_ignore_ascii_case(expected))
        }) {
            Self::Html
        } else if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("css")) {
            Self::Css
        } else if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
            Self::Json
        } else {
            Self::Other
        }
    }

    #[must_use]
    pub const fn file_type(self) -> FileType {
        match self {
            Self::Html => FileType::Html,
            Self::SourceMap => FileType::SourceMap,
            // CSS/JSON/JS/Other all analyze as JS-ish text; analyzers tolerate it.
            _ => FileType::Js,
        }
    }

    #[must_use]
    pub const fn is_text(self) -> bool {
        !matches!(self, Self::Other)
    }
}

/// One asset to ingest.
#[derive(Debug, Clone)]
pub struct IngestRequest {
    pub url: String,
    pub content_type: String,
    pub content: Vec<u8>,
    pub source: IngestionSource,
    /// Optional inline source-map content (Mindemon already fetched it).
    pub source_map_content: Option<String>,
    pub request_headers: Option<String>,
}

/// Result of ingesting one asset.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IngestResult {
    pub url: String,
    pub kind: String,
    pub file_id: i64,
    pub stored: bool,
    pub unchanged: bool,
    pub beautified_bytes: usize,
    pub findings: usize,
    pub reversed_sources: usize,
    pub relationships: usize,
    pub error: Option<String>,
}

/// Pipeline configuration.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Root directory where ingested files are written
    /// (`<workdir>/raw`, `<workdir>/beautified`, `<workdir>/reversed`).
    pub workdir: PathBuf,
    pub analyzer: AnalyzerConfig,
    /// Run the deobfuscator (not just the beautifier) on minified JS.
    pub deobfuscate: bool,
    /// Max bytes we attempt to beautify/analyze. `usize::MAX` (the default)
    /// means "no limit" — every text asset is processed regardless of size.
    /// Lower it only if you need to bound memory on constrained hosts.
    pub max_text_bytes: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            workdir: db::default_project_db_path("default")
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            analyzer: AnalyzerConfig::default(),
            deobfuscate: true,
            max_text_bytes: usize::MAX,
        }
    }
}

/// The shared ingestion pipeline.
pub struct Pipeline {
    db: ProjectDb,
    cfg: PipelineConfig,
    /// WebSocket broadcast sender (clients on :9222 subscribe). Optional so the
    /// pipeline can run headless (tests / CLI batch).
    ws_tx: Option<broadcast::Sender<WsMessage>>,
    hooks: HooksMap,
}

impl Pipeline {
    /// Build a pipeline over `db` writing under `cfg.workdir`.
    #[must_use]
    pub fn new(db: ProjectDb, cfg: PipelineConfig) -> Self {
        Self {
            db,
            cfg,
            ws_tx: None,
            hooks: HooksMap::new(),
        }
    }

    /// Attach a WebSocket broadcast sender so ingestion results stream to
    /// connected clients.
    #[must_use]
    pub fn with_ws(mut self, ws_tx: broadcast::Sender<WsMessage>) -> Self {
        self.ws_tx = Some(ws_tx);
        self
    }

    /// Attach hooks fired on ingestion events.
    #[must_use]
    pub fn with_hooks(mut self, hooks: HooksMap) -> Self {
        self.hooks = hooks;
        self
    }

    #[must_use]
    pub const fn db(&self) -> &ProjectDb {
        &self.db
    }

    fn broadcast(&self, msg: WsMessage) {
        if let Some(tx) = &self.ws_tx {
            // A send error only means there are currently no subscribers — fine.
            tx.send(msg).ok();
        }
    }

    /// Ingest one asset end-to-end. Never panics — failures are captured in
    /// [`IngestResult::error`].
    pub async fn ingest(&self, req: IngestRequest) -> IngestResult {
        match self.ingest_inner(req).await {
            Ok(r) => r,
            Err((url, e)) => IngestResult {
                url,
                error: Some(e),
                ..Default::default()
            },
        }
    }

    async fn ingest_inner(&self, req: IngestRequest) -> Result<IngestResult, (String, String)> {
        let url = req.url.clone();
        let kind = AssetKind::classify(&req.url, &req.content_type);
        let mut result = IngestResult {
            url: url.clone(),
            kind: format!("{kind:?}"),
            ..Default::default()
        };

        let rel_path = url_to_rel_path(&url, kind);

        // 1+2. Persist raw bytes + history-aware DB upsert.
        let file_id = self.store_raw(&url, kind, &req, &rel_path)?;
        result.file_id = file_id;
        result.stored = true;

        // Decode text (best-effort UTF-8) for analyzable kinds only.
        let text = if kind.is_text() && req.content.len() <= self.cfg.max_text_bytes {
            Some(String::from_utf8_lossy(&req.content).into_owned())
        } else {
            None
        };

        if let Some(src) = &text {
            result.beautified_bytes = self.beautify_step(kind, src, &rel_path);
            result.findings += self.analyze_step(&url, kind, src, file_id);
        }

        // Source-map reversal: an explicit map, or this asset if it *is* a map.
        let map_text = if kind == AssetKind::SourceMap {
            text.clone()
        } else {
            req.source_map_content.clone()
        };
        if let Some(map_json) = map_text {
            let (sources, extra_findings) = self.sourcemap_step(&url, &map_json, file_id);
            result.reversed_sources = sources;
            result.findings += extra_findings;
        }

        result.relationships = self.notify_step(&url, kind, file_id, text.as_deref(), &result).await;

        Ok(result)
    }

    /// Write raw bytes to disk and upsert the history-aware file row.
    fn store_raw(
        &self,
        url: &str,
        kind: AssetKind,
        req: &IngestRequest,
        rel_path: &Path,
    ) -> Result<i64, (String, String)> {
        let content_hash = ProjectDb::content_hash(&req.content);
        let raw_path = self.cfg.workdir.join("raw").join(rel_path);
        write_file(&raw_path, &req.content).map_err(|e| (url.to_string(), e))?;

        let fs_path = raw_path.to_string_lossy();
        let upsert = if kind == AssetKind::Html {
            db::files::upsert_html_file(
                &self.db,
                url,
                &fs_path,
                &content_hash,
                req.request_headers.as_deref(),
                req.source,
            )
        } else {
            db::files::upsert_js_file(
                &self.db,
                url,
                &fs_path,
                &content_hash,
                req.request_headers.as_deref(),
                req.source,
            )
        };
        upsert.map_err(|e| (url.to_string(), format!("db upsert: {e}")))
    }

    /// Beautify (and optionally deobfuscate) JS, writing the pretty output.
    /// Returns the beautified byte length (0 when not applicable).
    fn beautify_step(&self, kind: AssetKind, src: &str, rel_path: &Path) -> usize {
        if !matches!(kind, AssetKind::JavaScript) {
            return 0;
        }
        let opts = Options {
            deobfuscate: self.cfg.deobfuscate && looks_minified(src),
            ..Options::default()
        };
        let pretty_path = self.cfg.workdir.join("beautified").join(rel_path);
        // Never drop a file: write the beautified output when we have it, but
        // fall back to the raw source whenever beautification errors *or*
        // returns empty output for non-empty input (e.g. a deobfuscator pass
        // that produced nothing). Downstream consumers always find a copy at
        // the expected `beautified/` path.
        let out = match beautify(src, &opts) {
            Ok(out) if !out.trim().is_empty() || src.trim().is_empty() => out,
            Ok(_) => {
                eprintln!("[pipeline] beautify produced empty output; writing raw source as fallback");
                src.to_string()
            }
            Err(e) => {
                eprintln!("[pipeline] beautify failed ({e}); writing raw source as fallback");
                src.to_string()
            }
        };
        write_or_log(&pretty_path, out.as_bytes());
        out.len()
    }

    /// Run analyzers, persist matches, broadcast. Returns the finding count.
    fn analyze_step(&self, url: &str, kind: AssetKind, src: &str, file_id: i64) -> usize {
        let ft = kind.file_type();
        let matches = analyzers::run_all(src, Some(url), ft, &self.cfg.analyzer);
        if matches.is_empty() {
            return 0;
        }
        self.persist_matches(file_id, ft, &matches);
        self.broadcast(WsMessage::MatchesUpdate(
            serde_json::to_value(&matches).unwrap_or(serde_json::Value::Null),
        ));
        matches.len()
    }

    /// Reverse a source map: write each recovered original, re-analyze it.
    /// Returns `(reversed_source_count, extra_findings)`.
    fn sourcemap_step(&self, url: &str, map_json: &str, file_id: i64) -> (usize, usize) {
        let Ok(reversed) = crate::sourcemap::reverse(map_json) else {
            return (0, 0);
        };
        let mut extra_findings = 0;
        for rs in &reversed {
            let safe = rs.source_path.replace("..", "_");
            let dest = self.cfg.workdir.join("reversed").join(safe.trim_start_matches('/'));
            write_or_log(&dest, rs.content.as_bytes());
            let m = analyzers::run_all(
                &rs.content,
                Some(rs.source_path.as_str()),
                FileType::ReversedSource,
                &self.cfg.analyzer,
            );
            if !m.is_empty() {
                extra_findings += m.len();
                self.persist_matches(file_id, FileType::ReversedSource, &m);
            }
        }
        if !reversed.is_empty() {
            self.broadcast(WsMessage::ReversedSourceMapFilesUpdate(serde_json::json!({
                "generatedUrl": url,
                "sources": reversed.len(),
            })));
        }
        (reversed.len(), extra_findings)
    }

    /// Build the HTML relationship graph (when HTML), broadcast file updates,
    /// publish event-bus events, and fire hooks. Returns relationship count.
    async fn notify_step(
        &self,
        url: &str,
        kind: AssetKind,
        file_id: i64,
        text: Option<&str>,
        result: &IngestResult,
    ) -> usize {
        let is_html = kind == AssetKind::Html;
        let mut relationships = 0;
        if is_html {
            if let Some(src) = text {
                let graph = crate::relationships::AssetGraph::from_html(src, url);
                relationships = graph.relations.len();
                self.broadcast(WsMessage::HtmlFilesUpdate(
                    serde_json::to_value(&graph).unwrap_or(serde_json::Value::Null),
                ));
            }
        } else {
            self.broadcast(WsMessage::JsFilesUpdate(serde_json::json!({
                "url": url,
                "fileId": file_id,
                "kind": result.kind,
                "findings": result.findings,
                "beautifiedBytes": result.beautified_bytes,
            })));
        }

        let topic = if is_html { "html_file_saved" } else { "js_file_saved" };
        let payload = serde_json::json!({
            "url": url, "fileId": file_id, "kind": result.kind, "findings": result.findings,
        });
        if let Err(e) = db::event_bus::publish(&self.db, topic, &payload) {
            eprintln!("[pipeline] event_bus publish({topic}): {e}");
        }
        if !self.hooks.is_empty() {
            hooks::fire(&self.hooks, topic, &payload).await;
            if result.findings > 0 {
                hooks::fire(&self.hooks, "matches_created", &payload).await;
            }
        }
        relationships
    }

    fn persist_matches(&self, file_id: i64, ft: FileType, matches: &[AnalyzerMatch]) {
        if matches.is_empty() {
            return;
        }
        let ft_str = match ft {
            FileType::Js => "js",
            FileType::Html => "html",
            FileType::SourceMap => "source_map",
            FileType::ReversedSource => "reversed_source",
        };
        match db::matches::start_run(&self.db, "run_all", ft_str, file_id) {
            Ok(run_id) => {
                if let Err(e) = db::matches::insert_matches(&self.db, run_id, matches) {
                    eprintln!("[pipeline] insert_matches: {e}");
                }
                if let Err(e) = db::matches::finish_run(&self.db, run_id) {
                    eprintln!("[pipeline] finish_run: {e}");
                }
            }
            Err(e) => eprintln!("[pipeline] start_run: {e}"),
        }
    }
}

/// Write `bytes` to `path`, logging (not propagating) any I/O error. Used for
/// best-effort artifact writes where a failure shouldn't abort ingestion.
fn write_or_log(path: &Path, bytes: &[u8]) {
    if let Err(e) = write_file(path, bytes) {
        eprintln!("[pipeline] {e}");
    }
}

/// Heuristic: is this source minified (long lines, low newline density)?
fn looks_minified(src: &str) -> bool {
    if src.len() < 2_000 {
        return false;
    }
    let newlines = src.bytes().filter(|&b| b == b'\n').count().max(1);
    let avg_line = src.len() / newlines;
    avg_line > 200
}

/// Map a URL to a safe relative filesystem path `<host>/<path>`.
fn url_to_rel_path(url: &str, kind: AssetKind) -> PathBuf {
    let no_frag = url.split(['?', '#']).next().unwrap_or(url);
    let (host, path) = no_frag
        .strip_prefix("https://")
        .or_else(|| no_frag.strip_prefix("http://"))
        .map_or_else(
            || ("local".to_string(), no_frag.trim_start_matches('/').to_string()),
            |rest| match rest.split_once('/') {
                Some((h, p)) => (h.to_string(), p.to_string()),
                None => (rest.to_string(), String::new()),
            },
        );

    let mut out = PathBuf::from(sanitize_segment(&host));
    if path.is_empty() {
        out.push(default_index_name(kind));
    } else {
        for seg in path.split('/') {
            if seg.is_empty() || seg == "." {
                continue;
            }
            out.push(sanitize_segment(seg));
        }
    }
    // Ensure a filename with an extension exists.
    let needs_name = out
        .file_name()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.contains('.'));
    if needs_name {
        out.push(default_index_name(kind));
    }
    out
}

const fn default_index_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Html => "index.html",
        AssetKind::Css => "index.css",
        AssetKind::Json => "index.json",
        AssetKind::SourceMap => "index.map",
        _ => "index.js",
    }
}

fn sanitize_segment(seg: &str) -> String {
    let cleaned: String = seg
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').to_string();
    let out = if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed
    };
    out.chars().take(180).collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all({}): {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("write({}): {e}", path.display()))
}

/// Adapter implementing the proxy [`crate::proxy::Ingestor`] trait so the MITM
/// proxy feeds the same [`Pipeline`] as the HTTP API.
pub struct PipelineIngestor {
    pipeline: Arc<Pipeline>,
}

impl PipelineIngestor {
    #[must_use]
    pub const fn new(pipeline: Arc<Pipeline>) -> Self {
        Self { pipeline }
    }
}

#[async_trait::async_trait]
impl crate::proxy::Ingestor for PipelineIngestor {
    async fn on_exchange(&self, ex: crate::proxy::ProxyExchange) {
        let content_type = ex
            .response_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let kind = AssetKind::classify(&ex.url, &content_type);
        // Only ingest analyzable assets from proxy traffic.
        if matches!(kind, AssetKind::Other) {
            return;
        }
        let req = IngestRequest {
            url: ex.url,
            content_type,
            content: ex.response_body.to_vec(),
            source: IngestionSource::Proxy,
            source_map_content: None,
            request_headers: None,
        };
        let _ = self.pipeline.ingest(req).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{Ingestor, ProxyExchange};

    fn test_pipeline() -> Pipeline {
        let db = ProjectDb::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("jsb-pipeline-{}-{}", std::process::id(), rand_suffix()));
        let cfg = PipelineConfig {
            workdir: dir,
            ..PipelineConfig::default()
        };
        Pipeline::new(db, cfg)
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::SeqCst)
    }

    #[test]
    fn classify_js_and_html() {
        assert_eq!(AssetKind::classify("https://x/app.js", ""), AssetKind::JavaScript);
        assert_eq!(
            AssetKind::classify("https://x/", "text/html; charset=utf-8"),
            AssetKind::Html
        );
        assert_eq!(AssetKind::classify("https://x/app.css", ""), AssetKind::Css);
        assert_eq!(AssetKind::classify("https://x/app.js.map", ""), AssetKind::SourceMap);
        assert_eq!(AssetKind::classify("https://x/logo.png", "image/png"), AssetKind::Other);
    }

    #[test]
    fn url_to_path_is_safe() {
        let p = url_to_rel_path("https://example.com/a/../b/app.js", AssetKind::JavaScript);
        let s = p.to_string_lossy();
        assert!(s.starts_with("example.com/"));
        assert!(!s.contains(".."));
        assert!(s.ends_with("app.js"));
    }

    #[tokio::test]
    async fn ingest_js_stores_file_and_matches() {
        let p = test_pipeline();
        let js = r#"const SECRET_KEY="ghp_abc123def456ghi789jkl012mno345pqr6";fetch("/api/v2/users");"#;
        let res = p
            .ingest(IngestRequest {
                url: "https://victim.test/static/app.js".into(),
                content_type: "application/javascript".into(),
                content: js.as_bytes().to_vec(),
                source: IngestionSource::Cli,
                source_map_content: None,
                request_headers: None,
            })
            .await;
        assert!(res.error.is_none(), "ingest error: {:?}", res.error);
        assert!(res.stored);
        assert!(res.file_id > 0);

        // js_files row exists.
        let cur = db::files::current_js_file(p.db(), "https://victim.test/static/app.js")
            .unwrap()
            .expect("js_file row");
        assert_eq!(cur.id, res.file_id);

        // findings persisted.
        assert!(res.findings > 0, "expected analyzer findings");
        let stored = db::matches::get_matches(p.db(), &db::matches::MatchFilter::default()).unwrap();
        assert!(!stored.is_empty(), "expected stored matches in DB");
    }

    #[tokio::test]
    async fn ingest_sourcemap_reverses_sources() {
        let p = test_pipeline();
        let map = r#"{"version":3,"sources":["src/login.ts"],"sourcesContent":["export const API=\"/api/auth\";"],"names":[],"mappings":""}"#;
        let res = p
            .ingest(IngestRequest {
                url: "https://victim.test/static/app.js.map".into(),
                content_type: "application/json".into(),
                content: map.as_bytes().to_vec(),
                source: IngestionSource::Cli,
                source_map_content: None,
                request_headers: None,
            })
            .await;
        assert!(res.error.is_none(), "err: {:?}", res.error);
        assert_eq!(res.reversed_sources, 1);
    }

    #[tokio::test]
    async fn proxy_ingestor_persists_js() {
        let p = Arc::new(test_pipeline());
        let ing = PipelineIngestor::new(p.clone());
        ing.on_exchange(ProxyExchange {
            url: "https://victim.test/x/main.js".into(),
            method: "GET".into(),
            status: 200,
            request_headers: vec![],
            response_headers: vec![("content-type".into(), "application/javascript".into())],
            request_body: bytes::Bytes::new(),
            response_body: bytes::Bytes::from_static(b"var x=1;fetch('/api/me');"),
            is_https: true,
        })
        .await;
        let cur = db::files::current_js_file(p.db(), "https://victim.test/x/main.js").unwrap();
        assert!(cur.is_some(), "proxy exchange should persist a js_file");
    }
}
