use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::ca::CertificateAuthority;
use super::response::bad_gateway;
use super::server::ProxyServer;
use super::types::ProxyExchange;

pub(super) async fn tcp_relay<S>(client: S, host: String, port: u16) -> std::io::Result<()>
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

pub(super) async fn tls_intercept<S>(
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
                .map_or_else(|| "/".to_string(), |p| p.as_str().to_string());
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
                        if !is_hop_by_hop(k)
                            && let (Ok(n), Ok(v)) = (
                                hyper::header::HeaderName::from_bytes(k.as_bytes()),
                                hyper::header::HeaderValue::from_str(v),
                            )
                        {
                            b = b.header(n, v);
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
        if !is_hop_by_hop(k)
            && let (Ok(n), Ok(v)) = (
                hyper::header::HeaderName::from_bytes(k.as_bytes()),
                hyper::header::HeaderValue::from_str(v),
            )
        {
            req.headers_mut().insert(n, v);
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
    let body = collect_body(resp.into_body(), 16 * 1024 * 1024).await?;
    Ok((status, rh, body))
}

pub(super) async fn collect_body(body: Incoming, max: usize) -> Result<Bytes, String> {
    // CWE-789/400: enforce the cap WHILE streaming. `Limited` aborts the read
    // once `max` bytes have arrived, so a malicious/oversized (or endless
    // chunked) body can never be fully buffered into memory and OOM the daemon
    // — unlike `body.collect()` which buffers everything before any size check.
    use http_body_util::Limited;
    let collected = Limited::new(body, max)
        .collect()
        .await
        .map_err(|e| format!("body exceeds max ({max} bytes) or read error: {e}"))?;
    Ok(collected.to_bytes())
}

pub(super) fn is_hop_by_hop(name: &str) -> bool {
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
