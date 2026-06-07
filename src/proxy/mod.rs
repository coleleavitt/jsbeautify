//! MITM HTTP/HTTPS proxy.
//!
//! Custom from-scratch implementation (jxscout's own approach per
//! REPORT_06 §1 — they fork hudsucker but the layer here is built natively).
//!
//! ## Capabilities
//!
//! - HTTP forward proxy: incoming `GET http://host/path` → outbound `GET /path` to `host`.
//! - HTTPS via `CONNECT` tunneling, with optional TLS interception:
//!   - `intercept_tls = false`: pure passthrough (TCP relay), no body inspection.
//!   - `intercept_tls = true`: terminate TLS using a per-host leaf cert from
//!     [`ca::CertificateAuthority`], decrypt the request, ingest, and re-encrypt
//!     to the upstream.
//! - Scope filtering (delegates to [`crate::scope::ScopeMatcher`]).
//! - Optional ingestion bridge: every captured request/response is offered
//!   to a user-supplied [`Ingestor`] (typically writes to the project DB).
//!
//! ## Limits
//!
//! - HTTP/1.1 only (HTTP/2 + ALPN are out of scope here).
//! - Streaming bodies are buffered in full before being forwarded; large
//!   responses pay a memory cost. Tunable via `max_body_bytes`.
//!
//! ## Example
//!
//! ```no_run
//! # use js_beautify_rs::proxy::{ProxyServer, ProxyConfig, NoopIngestor};
//! # async fn run() {
//! let cfg = ProxyConfig {
//!     bind: "127.0.0.1:8080".parse().unwrap(),
//!     intercept_tls: true,
//!     ..ProxyConfig::default()
//! };
//! let server = ProxyServer::new(cfg, std::sync::Arc::new(NoopIngestor));
//! server.serve().await.unwrap();
//! # }
//! ```

pub mod ca;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::scope::ScopeMatcher;
use ca::CertificateAuthority;

/// User callback that receives every (request, response) pair the proxy sees.
#[async_trait::async_trait]
pub trait Ingestor: Send + Sync {
    async fn on_exchange(&self, exchange: ProxyExchange);
}

/// One captured request/response pair.
#[derive(Debug, Clone)]
pub struct ProxyExchange {
    pub url: String,
    pub method: String,
    pub status: u16,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request_body: Bytes,
    pub response_body: Bytes,
    pub is_https: bool,
}

/// No-op ingestor for tests / when only filtering is needed.
pub struct NoopIngestor;
#[async_trait::async_trait]
impl Ingestor for NoopIngestor {
    async fn on_exchange(&self, _: ProxyExchange) {}
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub bind: SocketAddr,
    pub intercept_tls: bool,
    pub ca_dir: std::path::PathBuf,
    pub max_body_bytes: usize,
    pub in_scope: Vec<String>,
    pub out_of_scope: Vec<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".parse().unwrap(),
            intercept_tls: false,
            ca_dir: ca::default_dir(),
            max_body_bytes: 16 * 1024 * 1024,
            in_scope: Vec::new(),
            out_of_scope: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct ProxyServer {
    cfg: Arc<ProxyConfig>,
    ca: Option<CertificateAuthority>,
    ingestor: Arc<dyn Ingestor>,
    scope: Arc<ScopeMatcher>,
}

impl ProxyServer {
    /// Construct. Loads/generates the CA if `intercept_tls=true`.
    #[must_use]
    pub fn new(cfg: ProxyConfig, ingestor: Arc<dyn Ingestor>) -> Self {
        let ca = if cfg.intercept_tls {
            CertificateAuthority::load_or_generate(&cfg.ca_dir).ok()
        } else {
            None
        };
        let in_scope: Vec<&str> = cfg.in_scope.iter().map(String::as_str).collect();
        let out_of_scope: Vec<&str> = cfg.out_of_scope.iter().map(String::as_str).collect();
        let scope = ScopeMatcher::new(&in_scope, &out_of_scope);
        Self {
            cfg: Arc::new(cfg),
            ca,
            ingestor,
            scope: Arc::new(scope),
        }
    }

    /// Bind and serve forever.
    ///
    /// # Errors
    /// Returns I/O errors on bind / accept.
    pub async fn serve(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.cfg.bind).await?;
        eprintln!("[proxy] listening on http://{}", self.cfg.bind);
        if self.cfg.intercept_tls {
            eprintln!("[proxy] TLS interception ON; CA at {}", self.cfg.ca_dir.display());
        }
        loop {
            let (stream, peer) = listener.accept().await?;
            let me = self.clone();
            tokio::spawn(async move {
                if let Err(e) = me.handle_client(stream, peer).await {
                    eprintln!("[proxy] {peer}: {e}");
                }
            });
        }
    }

    async fn handle_client(self, stream: TcpStream, _peer: SocketAddr) -> std::io::Result<()> {
        let io = TokioIo::new(stream);
        let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
            let me = self.clone();
            async move { me.dispatch(req).await }
        });
        if let Err(e) = hyper::server::conn::http1::Builder::new()
            .preserve_header_case(true)
            .title_case_headers(true)
            .serve_connection(io, svc)
            .with_upgrades()
            .await
        {
            // Connection-level error (closed early, etc.) — log but don't propagate.
            eprintln!("[proxy] connection error: {e}");
        }
        Ok(())
    }

    async fn dispatch(self, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
        if req.method() == hyper::Method::CONNECT {
            return Ok(self.handle_connect(req).await);
        }
        Ok(self.handle_http(req).await)
    }

    async fn handle_http(self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        // For plain HTTP forward-proxy mode the URI is absolute.
        let uri = req.uri().clone();
        let host = match uri.host() {
            Some(h) => h.to_string(),
            None => return bad_request("missing host in proxy request"),
        };
        if !self.scope.is_in_scope(&host) {
            // Still forward, but skip ingestion.
            return self.forward_http(req, false).await;
        }
        self.forward_http(req, true).await
    }

    async fn forward_http(self, req: Request<Incoming>, ingest: bool) -> Response<Full<Bytes>> {
        let method = req.method().clone();
        let uri = req.uri().clone();
        let url_str = uri.to_string();

        let req_headers: Vec<(String, String)> = req
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let host = uri.host().unwrap_or("").to_string();
        let port = uri.port_u16().unwrap_or(80);
        let path = uri
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());

        // Buffer request body
        let req_body = match collect_body(req.into_body(), self.cfg.max_body_bytes).await {
            Ok(b) => b,
            Err(e) => return bad_request(&format!("read request body: {e}")),
        };

        // Connect upstream over plain TCP
        let upstream = match TcpStream::connect(format!("{host}:{port}")).await {
            Ok(s) => s,
            Err(e) => return bad_gateway(&format!("upstream connect: {e}")),
        };

        let (tx_io, conn) = match hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(upstream)).await
        {
            Ok(p) => p,
            Err(e) => return bad_gateway(&format!("hyper handshake: {e}")),
        };
        tokio::spawn(conn);
        let mut tx_io = tx_io;

        let mut up_req = Request::builder()
            .method(method.clone())
            .uri(path)
            .body(Full::new(req_body.clone()))
            .unwrap_or_else(|_| Request::new(Full::new(Bytes::new())));
        // Re-attach headers (skip hop-by-hop)
        for (k, v) in &req_headers {
            if !is_hop_by_hop(k) {
                if let (Ok(name), Ok(value)) = (
                    hyper::header::HeaderName::from_bytes(k.as_bytes()),
                    hyper::header::HeaderValue::from_str(v),
                ) {
                    up_req.headers_mut().insert(name, value);
                }
            }
        }
        // Set Host header explicitly.
        if let Ok(h) = hyper::header::HeaderValue::from_str(&host) {
            up_req.headers_mut().insert(hyper::header::HOST, h);
        }

        let resp = match tx_io.send_request(up_req).await {
            Ok(r) => r,
            Err(e) => return bad_gateway(&format!("upstream send: {e}")),
        };
        let status = resp.status();
        let resp_headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let resp_body = match collect_body(resp.into_body(), self.cfg.max_body_bytes).await {
            Ok(b) => b,
            Err(e) => return bad_gateway(&format!("read response body: {e}")),
        };

        if ingest {
            self.ingestor
                .on_exchange(ProxyExchange {
                    url: url_str,
                    method: method.to_string(),
                    status: status.as_u16(),
                    request_headers: req_headers,
                    response_headers: resp_headers.clone(),
                    request_body: req_body,
                    response_body: resp_body.clone(),
                    is_https: false,
                })
                .await;
        }

        let mut builder = Response::builder().status(status);
        for (k, v) in &resp_headers {
            if !is_hop_by_hop(k) {
                if let (Ok(name), Ok(value)) = (
                    hyper::header::HeaderName::from_bytes(k.as_bytes()),
                    hyper::header::HeaderValue::from_str(v),
                ) {
                    builder = builder.header(name, value);
                }
            }
        }
        builder
            .body(Full::new(resp_body))
            .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
    }

    async fn handle_connect(self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        // The CONNECT target is in the URI authority.
        let auth = match req.uri().authority() {
            Some(a) => a.clone(),
            None => return bad_request("CONNECT requires authority"),
        };
        let host = auth.host().to_string();
        let port = auth.port_u16().unwrap_or(443);

        let upgrade = hyper::upgrade::on(req);
        let me = self.clone();

        tokio::spawn(async move {
            let upgraded = match upgrade.await {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("[proxy] CONNECT upgrade failed: {e}");
                    return;
                }
            };
            let upgraded = TokioIo::new(upgraded);

            if me.cfg.intercept_tls && me.scope.is_in_scope(&host) {
                if let Some(ca) = &me.ca {
                    if let Err(e) = tls_intercept(upgraded, host.clone(), port, ca.clone(), me.clone()).await {
                        eprintln!("[proxy] tls intercept {host}:{port}: {e}");
                    }
                    return;
                }
            }
            // Passthrough
            if let Err(e) = tcp_relay(upgraded, host.clone(), port).await {
                eprintln!("[proxy] tcp relay {host}:{port}: {e}");
            }
        });

        Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::new()))
            .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
    }
}

async fn tcp_relay<S>(client: S, host: String, port: u16) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let upstream = TcpStream::connect(format!("{host}:{port}")).await?;
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut ur, mut uw) = tokio::io::split(upstream);
    let c2u = async {
        tokio::io::copy(&mut cr, &mut uw).await?;
        uw.shutdown().await
    };
    let u2c = async {
        tokio::io::copy(&mut ur, &mut cw).await?;
        cw.shutdown().await
    };
    let _ = tokio::try_join!(c2u, u2c);
    Ok(())
}

async fn tls_intercept<S>(
    client: S,
    host: String,
    port: u16,
    ca: CertificateAuthority,
    server: ProxyServer,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    use tokio_rustls::TlsAcceptor;

    let (chain, key) = ca.leaf_for(&host)?;
    let server_cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| format!("rustls server cfg: {e}"))?;
    let acceptor = TlsAcceptor::from(Arc::new(server_cfg));
    let tls_client = acceptor.accept(client).await.map_err(|e| format!("accept: {e}"))?;

    // Now run an HTTP/1 server on the decrypted client stream.
    let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
        let host = host.clone();
        let server = server.clone();
        async move {
            // For TLS-intercepted CONNECT, the request URI is path-only;
            // we inject the absolute URL for ingestion.
            let path = req
                .uri()
                .path_and_query()
                .map(|p| p.as_str().to_string())
                .unwrap_or_else(|| "/".to_string());
            let url = format!("https://{host}:{port}{path}");
            let method = req.method().clone();
            let req_headers: Vec<(String, String)> = req
                .headers()
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let req_body = collect_body(req.into_body(), server.cfg.max_body_bytes)
                .await
                .map_err(|e| format!("read body: {e}"))
                .unwrap_or_default();

            // Connect upstream over TLS
            let resp = upstream_https(&host, port, &method, &path, &req_headers, &req_body).await;
            match resp {
                Ok((status, resp_headers, resp_body)) => {
                    if server.scope.is_in_scope(&host) {
                        server
                            .ingestor
                            .on_exchange(ProxyExchange {
                                url: url.clone(),
                                method: method.to_string(),
                                status: status.as_u16(),
                                request_headers: req_headers,
                                response_headers: resp_headers.clone(),
                                request_body: req_body,
                                response_body: resp_body.clone(),
                                is_https: true,
                            })
                            .await;
                    }
                    let mut b = Response::builder().status(status);
                    for (k, v) in &resp_headers {
                        if !is_hop_by_hop(k) {
                            if let (Ok(n), Ok(v)) = (
                                hyper::header::HeaderName::from_bytes(k.as_bytes()),
                                hyper::header::HeaderValue::from_str(v),
                            ) {
                                b = b.header(n, v);
                            }
                        }
                    }
                    Ok::<_, hyper::Error>(
                        b.body(Full::new(resp_body))
                            .unwrap_or_else(|_| Response::new(Full::new(Bytes::new()))),
                    )
                }
                Err(e) => Ok(bad_gateway(&format!("tls upstream: {e}"))),
            }
        }
    });

    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(tls_client), svc)
        .await
    {
        return Err(format!("intercepted serve: {e}"));
    }
    Ok(())
}

async fn upstream_https(
    host: &str,
    port: u16,
    method: &hyper::Method,
    path: &str,
    headers: &[(String, String)],
    body: &Bytes,
) -> Result<(StatusCode, Vec<(String, String)>, Bytes), String> {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, RootCertStore};
    use tokio_rustls::TlsConnector;
    use webpki_roots::TLS_SERVER_ROOTS;

    let mut roots = RootCertStore::empty();
    roots.extend(TLS_SERVER_ROOTS.iter().cloned());
    let cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));
    let stream = TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| e.to_string())?;
    let dns = ServerName::try_from(host.to_string()).map_err(|e| format!("dns: {e}"))?;
    let tls = connector.connect(dns, stream).await.map_err(|e| e.to_string())?;

    let (sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(tls))
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(conn);
    let mut sender = sender;

    let mut req = Request::builder()
        .method(method.clone())
        .uri(path)
        .body(Full::new(body.clone()))
        .map_err(|e| e.to_string())?;
    for (k, v) in headers {
        if !is_hop_by_hop(k) {
            if let (Ok(n), Ok(v)) = (
                hyper::header::HeaderName::from_bytes(k.as_bytes()),
                hyper::header::HeaderValue::from_str(v),
            ) {
                req.headers_mut().insert(n, v);
            }
        }
    }
    if let Ok(h) = hyper::header::HeaderValue::from_str(host) {
        req.headers_mut().insert(hyper::header::HOST, h);
    }

    let resp = sender.send_request(req).await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let rh: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = collect_body(resp.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|e| e.to_string())?;
    Ok((status, rh, body))
}

async fn collect_body(body: Incoming, max: usize) -> Result<Bytes, String> {
    let collected = body.collect().await.map_err(|e| e.to_string())?;
    let bytes = collected.to_bytes();
    if bytes.len() > max {
        return Err(format!("body exceeds max ({} > {})", bytes.len(), max));
    }
    Ok(bytes)
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "proxy-connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

fn bad_request(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(Full::new(Bytes::from(format!("400 Bad Request: {msg}"))))
        .expect("static response always builds")
}

fn bad_gateway(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(Full::new(Bytes::from(format!("502 Bad Gateway: {msg}"))))
        .expect("static response always builds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn config_default_binds_loopback() {
        let cfg = ProxyConfig::default();
        assert!(cfg.bind.ip().is_loopback());
        assert_eq!(cfg.bind.port(), 8080);
        assert!(!cfg.intercept_tls);
    }

    #[test]
    fn hop_by_hop_classification() {
        assert!(is_hop_by_hop("Connection"));
        assert!(is_hop_by_hop("transfer-encoding"));
        assert!(!is_hop_by_hop("Content-Type"));
    }

    /// End-to-end smoke test: spin up a tiny HTTP origin, point the proxy at it,
    /// and verify the proxy forwards the request and ingests the exchange.
    #[test]
    fn proxy_forwards_http_and_ingests() {
        use std::sync::Mutex;
        use tokio::runtime::Runtime;

        struct CapturingIngestor(Arc<Mutex<Vec<ProxyExchange>>>);
        #[async_trait::async_trait]
        impl Ingestor for CapturingIngestor {
            async fn on_exchange(&self, ex: ProxyExchange) {
                self.0.lock().unwrap().push(ex);
            }
        }

        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            // 1. Origin server: returns "hello\n" for any GET.
            let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin_addr = origin.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let (mut s, _) = origin.accept().await.unwrap();
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 8192];
                        let _ = s.read(&mut buf).await;
                        let body = "hello\n";
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = s.write_all(resp.as_bytes()).await;
                        let _ = s.shutdown().await;
                    });
                }
            });

            // 2. Proxy listening on a random loopback port.
            let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let proxy_addr = proxy_listener.local_addr().unwrap();
            drop(proxy_listener);

            let captured = Arc::new(Mutex::new(Vec::<ProxyExchange>::new()));
            let cfg = ProxyConfig {
                bind: proxy_addr,
                in_scope: vec!["127.0.0.1".to_string()],
                ..ProxyConfig::default()
            };
            let server = ProxyServer::new(cfg, Arc::new(CapturingIngestor(captured.clone())));
            tokio::spawn(server.serve());
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            // 3. Send `GET http://127.0.0.1:<origin_port>/x` to the proxy.
            let mut s = TcpStream::connect(proxy_addr).await.unwrap();
            let req = format!(
                "GET http://127.0.0.1:{}/x HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
                origin_addr.port()
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).await.unwrap();
            let resp_text = String::from_utf8_lossy(&resp);
            assert!(resp_text.contains("200 OK"), "expected 200, got: {resp_text}");
            assert!(resp_text.contains("hello"), "expected body 'hello', got: {resp_text}");

            // 4. Ingestor saw it.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let captured = captured.lock().unwrap();
            assert_eq!(captured.len(), 1, "expected 1 ingested exchange");
            assert_eq!(captured[0].status, 200);
            assert!(captured[0].url.contains("/x"));
        });
    }
}
