use bytes::Bytes;
use std::net::SocketAddr;

use super::ca;

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
