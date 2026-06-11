//! HTTP ingestion API (the jxscout `:3333` server, reimplemented natively).
//!
//! This is the front door for the **Mindemon** Firefox DevTools extension: the
//! panel captures every JS/CSS/source-map/HTML asset a page loads and `POST`s
//! them here, where each one flows through the shared [`crate::pipeline::Pipeline`]
//! (beautify → deobfuscate → analyze → sourcemap-reverse → DB → WebSocket).
//!
//! ## Routes
//!
//! | Method | Path            | Purpose |
//! |--------|-----------------|---------|
//! | GET    | `/health`       | Liveness + version. |
//! | GET    | `/ca.crt`       | The MITM CA PEM, so a browser can trust the proxy. |
//! | POST   | `/ingest`       | One asset (see [`IngestBody`]). |
//! | POST   | `/ingest/batch` | A Mindemon dump: `{ target, files: [IngestBody, …] }`. |
//! | GET    | `/matches`      | Recent stored matches as JSON. |
//! | OPTIONS| `*`             | CORS preflight. |
//!
//! Every response carries permissive CORS headers so the `moz-extension://`
//! devtools panel (a distinct origin) can call it.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::db::files::IngestionSource;
use crate::db::matches::{MatchFilter, get_matches};
use crate::pipeline::{IngestRequest, Pipeline};

pub mod daemon;
pub use daemon::{ServeConfig, serve};

/// One asset in an ingestion request. Field names match the JSON the Mindemon
/// panel emits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestBody {
    pub url: String,
    #[serde(default)]
    pub content_type: String,
    /// Asset body. UTF-8 text by default, or base64 when `encoding == "base64"`.
    pub content: String,
    /// `"utf8"` (default) or `"base64"`.
    #[serde(default)]
    pub encoding: Option<String>,
    /// Optional already-fetched source-map JSON (Mindemon provides it).
    #[serde(default)]
    pub source_map_content: Option<String>,
    /// Originating subsystem label; defaults to the HTTP API.
    #[serde(default)]
    pub source: Option<String>,
}

impl IngestBody {
    fn decode_content(&self) -> Result<Vec<u8>, String> {
        match self.encoding.as_deref() {
            Some("base64") => base64::engine::general_purpose::STANDARD
                .decode(self.content.trim())
                .map_err(|e| format!("base64 decode: {e}")),
            _ => Ok(self.content.clone().into_bytes()),
        }
    }

    fn into_ingest_request(self) -> Result<IngestRequest, String> {
        let content = self.decode_content()?;
        let source = match self.source.as_deref() {
            Some("proxy") | Some("PROXY") => IngestionSource::Proxy,
            Some("cli") | Some("CLI") => IngestionSource::Cli,
            _ => IngestionSource::FsIngest,
        };
        Ok(IngestRequest {
            url: self.url,
            content_type: self.content_type,
            content,
            source,
            source_map_content: self.source_map_content,
            request_headers: None,
        })
    }
}

/// Batch ingestion payload — the shape of a full Mindemon dump.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchBody {
    #[serde(default)]
    pub target: Option<String>,
    pub files: Vec<IngestBody>,
}

/// HTTP API server config.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub bind: SocketAddr,
    /// CA certificate PEM served at `/ca.crt` (when TLS interception is on).
    pub ca_pem: Option<String>,
    pub max_body_bytes: usize,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:3333".parse().expect("valid default addr"),
            ca_pem: None,
            max_body_bytes: 64 * 1024 * 1024,
        }
    }
}

/// The ingestion API server. Cheap to clone (`Arc` interior).
#[derive(Clone)]
pub struct ApiServer {
    cfg: Arc<ApiConfig>,
    pipeline: Arc<Pipeline>,
}

impl ApiServer {
    #[must_use]
    pub fn new(cfg: ApiConfig, pipeline: Arc<Pipeline>) -> Self {
        Self {
            cfg: Arc::new(cfg),
            pipeline,
        }
    }

    /// Bind and serve forever.
    ///
    /// # Errors
    /// I/O errors on bind / accept.
    pub async fn serve(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.cfg.bind).await?;
        eprintln!("[api] ingestion API on http://{}", self.cfg.bind);
        loop {
            let (stream, _peer) = listener.accept().await?;
            let me = self.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req| {
                    let me = me.clone();
                    async move { Ok::<_, Infallible>(me.route(req).await) }
                });
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await
                {
                    eprintln!("[api] connection error: {e}");
                }
            });
        }
    }

    async fn route(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        if method == Method::OPTIONS {
            return cors(no_content());
        }

        let resp = match (&method, path.as_str()) {
            (&Method::GET, "/health") => json_ok(&serde_json::json!({
                "ok": true,
                "service": "js-beautify-rs",
                "version": env!("CARGO_PKG_VERSION"),
            })),
            (&Method::GET, "/ca.crt") => self.serve_ca(),
            (&Method::POST, "/ingest") => self.handle_ingest(req).await,
            (&Method::POST, "/ingest/batch") => self.handle_batch(req).await,
            (&Method::GET, "/matches") => self.handle_matches(),
            _ => error(StatusCode::NOT_FOUND, "not found"),
        };
        cors(resp)
    }

    fn serve_ca(&self) -> Response<Full<Bytes>> {
        match &self.cfg.ca_pem {
            Some(pem) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/x-x509-ca-cert")
                .header("content-disposition", "attachment; filename=\"js-beautify-rs-ca.crt\"")
                .body(Full::new(Bytes::from(pem.clone())))
                .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "ca build")),
            None => error(StatusCode::NOT_FOUND, "TLS interception disabled; no CA"),
        }
    }

    async fn handle_ingest(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        let body = match self.read_body(req).await {
            Ok(b) => b,
            Err(resp) => return resp,
        };
        let parsed: IngestBody = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("invalid JSON: {e}")),
        };
        let ingest = match parsed.into_ingest_request() {
            Ok(r) => r,
            Err(e) => return error(StatusCode::BAD_REQUEST, &e),
        };
        let result = self.pipeline.ingest(ingest).await;
        json_ok(&result)
    }

    async fn handle_batch(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        let body = match self.read_body(req).await {
            Ok(b) => b,
            Err(resp) => return resp,
        };
        let parsed: BatchBody = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("invalid JSON: {e}")),
        };
        let mut results = Vec::with_capacity(parsed.files.len());
        let mut total_findings = 0usize;
        for file in parsed.files {
            match file.into_ingest_request() {
                Ok(r) => {
                    let res = self.pipeline.ingest(r).await;
                    total_findings += res.findings;
                    results.push(res);
                }
                Err(e) => eprintln!("[api] skip batch item: {e}"),
            }
        }
        json_ok(&serde_json::json!({
            "target": parsed.target,
            "ingested": results.len(),
            "totalFindings": total_findings,
            "results": results,
        }))
    }

    fn handle_matches(&self) -> Response<Full<Bytes>> {
        let filter = MatchFilter {
            limit: Some(500),
            ..MatchFilter::default()
        };
        match get_matches(self.pipeline.db(), &filter) {
            Ok(matches) => json_ok(&matches),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("query matches: {e}")),
        }
    }

    async fn read_body(&self, req: Request<Incoming>) -> Result<Bytes, Response<Full<Bytes>>> {
        let collected = req
            .into_body()
            .collect()
            .await
            .map_err(|e| error(StatusCode::BAD_REQUEST, &format!("read body: {e}")))?;
        let bytes = collected.to_bytes();
        if bytes.len() > self.cfg.max_body_bytes {
            return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "body too large"));
        }
        Ok(bytes)
    }
}

// ── Response helpers ─────────────────────────────────────────────────────────

fn cors(mut resp: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let h = resp.headers_mut();
    h.insert("access-control-allow-origin", "*".parse().expect("static"));
    h.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().expect("static"),
    );
    h.insert("access-control-allow-headers", "content-type".parse().expect("static"));
    resp
}

fn json_ok<T: Serialize>(value: &T) -> Response<Full<Bytes>> {
    match serde_json::to_vec(value) {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "build response")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("serialize: {e}")),
    }
}

fn no_content() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .expect("static response always builds")
}

fn error(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({ "ok": false, "error": msg }).to_string();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ProjectDb;
    use crate::pipeline::PipelineConfig;

    fn test_server() -> (ApiServer, Arc<Pipeline>) {
        let db = ProjectDb::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("jsb-api-{}", std::process::id()));
        let pipeline = Arc::new(Pipeline::new(
            db,
            PipelineConfig {
                workdir: dir,
                ..PipelineConfig::default()
            },
        ));
        let server = ApiServer::new(ApiConfig::default(), pipeline.clone());
        (server, pipeline)
    }

    #[test]
    fn ingest_body_decodes_base64() {
        let b = IngestBody {
            url: "https://x/a.js".into(),
            content_type: "application/javascript".into(),
            content: base64::engine::general_purpose::STANDARD.encode("var x=1;"),
            encoding: Some("base64".into()),
            source_map_content: None,
            source: None,
        };
        assert_eq!(b.decode_content().unwrap(), b"var x=1;");
    }

    #[tokio::test]
    async fn end_to_end_ingest_over_http() {
        let (server, pipeline) = test_server();

        // Bind on an ephemeral port and spawn the server.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let mut cfg = ApiConfig::default();
        cfg.bind = addr;
        let server = ApiServer {
            cfg: Arc::new(cfg),
            pipeline: server.pipeline.clone(),
        };
        tokio::spawn(server.serve());
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        // POST a JS asset with a secret to /ingest.
        let body = serde_json::json!({
            "url": "https://victim.test/app.js",
            "content_type": "application/javascript",
            "content": "const SECRET_KEY=\"ghp_abc123def456ghi789jkl012mno345pqr6\";fetch('/api/v2/users');",
        })
        .to_string();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{addr}/ingest"))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let json: serde_json::Value = resp.json().await.unwrap();
        assert!(
            json["stored"].as_bool().unwrap_or(false),
            "expected stored=true: {json}"
        );
        assert!(json["findings"].as_u64().unwrap_or(0) > 0, "expected findings: {json}");

        // The pipeline persisted the file.
        let cur = crate::db::files::current_js_file(pipeline.db(), "https://victim.test/app.js").unwrap();
        assert!(cur.is_some());

        // /health works.
        let h = client.get(format!("http://{addr}/health")).send().await.unwrap();
        assert_eq!(h.status(), 200);
    }

    #[tokio::test]
    async fn options_preflight_has_cors() {
        let (server, _p) = test_server();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let server = ApiServer {
            cfg: Arc::new(ApiConfig {
                bind: addr,
                ..ApiConfig::default()
            }),
            pipeline: server.pipeline.clone(),
        };
        tokio::spawn(server.serve());
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;

        let client = reqwest::Client::new();
        let resp = client
            .request(reqwest::Method::OPTIONS, format!("http://{addr}/ingest"))
            .send()
            .await
            .unwrap();
        assert!(resp.headers().contains_key("access-control-allow-origin"));
    }
}
