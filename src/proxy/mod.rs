//! MITM HTTP/HTTPS proxy.
//!
//! Custom from-scratch implementation (jxscout's own approach per
//! `REPORT_06` §1 — they fork hudsucker but the layer here is built natively).
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

mod response;
mod server;
#[cfg(test)]
mod tests;
mod transport;
mod types;

pub use server::ProxyServer;
pub use types::{Ingestor, NoopIngestor, ProxyConfig, ProxyExchange};
