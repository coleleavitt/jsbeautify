//! HTTP ingestion API (the jxscout `:3333` server, reimplemented natively).
//!
//! This is the front door for the **Mindemon** Firefox `DevTools` extension: the
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

pub mod daemon;

mod api;
mod body;
mod response;
#[cfg(test)]
mod tests;

pub use api::{ApiConfig, ApiServer};
pub use body::{BatchBody, IngestBody};
pub use daemon::{ServeConfig, serve};
