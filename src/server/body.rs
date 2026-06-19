use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::db::files::IngestionSource;
use crate::pipeline::IngestRequest;

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
    pub(super) fn decode_content(&self) -> Result<Vec<u8>, String> {
        match self.encoding.as_deref() {
            Some("base64") => base64::engine::general_purpose::STANDARD
                .decode(self.content.trim())
                .map_err(|e| format!("base64 decode: {e}")),
            _ => Ok(self.content.clone().into_bytes()),
        }
    }

    pub(super) fn into_ingest_request(self) -> Result<IngestRequest, String> {
        let content = self.decode_content()?;
        let source = match self.source.as_deref() {
            Some("proxy" | "PROXY") => IngestionSource::Proxy,
            Some("cli" | "CLI") => IngestionSource::Cli,
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
