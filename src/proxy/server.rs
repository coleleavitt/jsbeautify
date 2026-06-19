use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

use crate::scope::ScopeMatcher;

use super::ca::CertificateAuthority;
use super::response::{bad_gateway, bad_request};
use super::transport::{collect_body, is_hop_by_hop, tcp_relay, tls_intercept};
use super::types::{Ingestor, ProxyConfig, ProxyExchange};

#[derive(Clone)]
pub struct ProxyServer {
    pub(super) cfg: Arc<ProxyConfig>,
    pub(super) ca: Option<CertificateAuthority>,
    pub(super) ingestor: Arc<dyn Ingestor>,
    pub(super) scope: Arc<ScopeMatcher>,
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
            return Ok(self.handle_connect(req));
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
            .map_or_else(|| "/".to_string(), |pq| pq.as_str().to_string());

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
            if !is_hop_by_hop(k)
                && let (Ok(name), Ok(value)) = (
                    hyper::header::HeaderName::from_bytes(k.as_bytes()),
                    hyper::header::HeaderValue::from_str(v),
                )
            {
                up_req.headers_mut().insert(name, value);
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
            if !is_hop_by_hop(k)
                && let (Ok(name), Ok(value)) = (
                    hyper::header::HeaderName::from_bytes(k.as_bytes()),
                    hyper::header::HeaderValue::from_str(v),
                )
            {
                builder = builder.header(name, value);
            }
        }
        builder
            .body(Full::new(resp_body))
            .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
    }

    fn handle_connect(self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        // The CONNECT target is in the URI authority.
        let auth = match req.uri().authority() {
            Some(a) => a.clone(),
            None => return bad_request("CONNECT requires authority"),
        };
        let host = auth.host().to_string();
        let port = auth.port_u16().unwrap_or(443);

        let upgrade = hyper::upgrade::on(req);
        let me = self;

        tokio::spawn(async move {
            let upgraded = match upgrade.await {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("[proxy] CONNECT upgrade failed: {e}");
                    return;
                }
            };
            let upgraded = TokioIo::new(upgraded);

            if me.cfg.intercept_tls
                && me.scope.is_in_scope(&host)
                && let Some(ca) = &me.ca
            {
                if let Err(e) = tls_intercept(upgraded, host.clone(), port, ca.clone(), me.clone()).await {
                    eprintln!("[proxy] tls intercept {host}:{port}: {e}");
                }
                return;
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
