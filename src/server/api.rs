use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use crate::db::matches::{MatchFilter, get_matches};
use crate::pipeline::Pipeline;

use super::body::{BatchBody, IngestBody};
use super::response::{cors, error, json_ok, no_content};

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
    pub(super) cfg: Arc<ApiConfig>,
    pub(super) pipeline: Arc<Pipeline>,
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
        self.cfg.ca_pem.as_ref().map_or_else(
            || error(StatusCode::NOT_FOUND, "TLS interception disabled; no CA"),
            |pem| {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/x-x509-ca-cert")
                    .header("content-disposition", "attachment; filename=\"js-beautify-rs-ca.crt\"")
                    .body(Full::new(Bytes::from(pem.clone())))
                    .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "ca build"))
            },
        )
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
