use super::transport::is_hop_by_hop;
use super::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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
