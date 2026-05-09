//! VS Code extension WebSocket API server.
//!
//! Ports `crates/core/proxy-api/src/websocket/` from jxscout-pro-v2
//! (REPORT_06 §8). Broadcasts real-time events to connected VS Code / Cursor
//! extension clients over a WebSocket connection.
//!
//! ## Protocol
//!
//! Messages are JSON objects with a `type` field:
//! ```json
//! { "type": "matches_update", "data": [...] }
//! ```
//!
//! 23+ message types per REPORT_06 §8.

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// All known message types from jxscout's VS Code extension protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum WsMessage {
    // ── Query responses ─────────────────────────────────────────────────
    GetBookmarksResponse(serde_json::Value),
    DeleteBookmarkResponse(serde_json::Value),
    BookmarkGroupsUpdate(serde_json::Value),
    BookmarkGroupsDelete(serde_json::Value),
    GetBookmarkGroupsResponse(serde_json::Value),
    GetProjectSettingsResponse(serde_json::Value),
    UntrackFilesResponse(serde_json::Value),

    // ── File events ─────────────────────────────────────────────────────
    JsFilesUpdate(serde_json::Value),
    JsFilesDelete(serde_json::Value),
    HtmlFilesUpdate(serde_json::Value),
    HtmlFilesDelete(serde_json::Value),
    ReversedSourceMapFilesUpdate(serde_json::Value),

    // ── Matches events ──────────────────────────────────────────────────
    MatchesUpdate(serde_json::Value),
    MatchesDelete(serde_json::Value),
    MarkMatchesSeenResponse(serde_json::Value),
    MarkMatchesUnseenResponse(serde_json::Value),
    MarkMatchesUninterestingResponse(serde_json::Value),

    // ── Task events ─────────────────────────────────────────────────────
    CancelTaskResponse(serde_json::Value),
    TaskStatusResponse(serde_json::Value),

    // ── Misc ────────────────────────────────────────────────────────────
    RefreshSitemapResponse(serde_json::Value),
    GetAssetRelationshipsResponse(serde_json::Value),

    // ── Generic broadcast ───────────────────────────────────────────────
    Broadcast(serde_json::Value),
}

impl WsMessage {
    /// Serialize to a JSON text frame.
    #[must_use]
    pub fn to_frame(&self) -> Message {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        Message::Text(json.into())
    }
}

/// Configuration for the WebSocket server.
#[derive(Debug, Clone)]
pub struct WsConfig {
    pub bind: SocketAddr,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:9222".parse().unwrap(),
        }
    }
}

/// The WebSocket API server. Spawns a tokio task that accepts connections
/// and broadcasts events to all connected clients.
pub struct WsServer {
    config: WsConfig,
    /// Broadcast channel — any subsystem can send events.
    pub tx: broadcast::Sender<WsMessage>,
}

impl WsServer {
    /// Create a new server. The [`broadcast::Sender`] is exposed so that
    /// the proxy, analyzer, monitoring, etc. subsystems can push events.
    #[must_use]
    pub fn new(config: WsConfig) -> Self {
        let (tx, _) = broadcast::channel(512);
        Self { config, tx }
    }

    /// Bind and accept connections forever. Typically spawned via `tokio::spawn`.
    ///
    /// # Errors
    /// I/O errors on bind.
    pub async fn serve(self: Arc<Self>) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.config.bind).await?;
        eprintln!("[ws] listening on ws://{}", self.config.bind);

        loop {
            let (stream, peer) = listener.accept().await?;
            let me = self.clone();
            tokio::spawn(async move {
                let ws = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        eprintln!("[ws] accept {peer}: {e}");
                        return;
                    }
                };
                eprintln!("[ws] client connected: {peer}");
                me.handle_connection(ws, peer).await;
                eprintln!("[ws] client disconnected: {peer}");
            });
        }
    }

    async fn handle_connection(
        &self,
        ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        _peer: SocketAddr,
    ) {
        let (mut sink, mut stream) = ws.split();
        let mut rx = self.tx.subscribe();

        // Forward broadcast events to the client.
        let send_task = tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                if sink.send(msg.to_frame()).await.is_err() {
                    break;
                }
            }
        });

        // Read client messages (we mostly ignore them for now; jxscout uses
        // request-response pattern where client sends a request type and we
        // reply with the corresponding response type). Placeholder loop.
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(text) => {
                    // Attempt to parse incoming client request. For now just log.
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(req_type) = v.get("type").and_then(|t| t.as_str()) {
                            eprintln!("[ws] client request: {req_type}");
                            // TODO: dispatch to handler based on req_type
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        send_task.abort();
    }

    /// Convenience: broadcast a message to all connected clients.
    pub fn broadcast(&self, msg: WsMessage) {
        let _ = self.tx.send(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_message_serializes() {
        let msg = WsMessage::MatchesUpdate(serde_json::json!([{"id": 1}]));
        let frame = msg.to_frame();
        let text = match frame {
            Message::Text(t) => t.to_string(),
            _ => panic!("expected text frame"),
        };
        assert!(text.contains("matches_update"));
        assert!(text.contains(r#""id":1"#));
    }

    #[test]
    fn ws_config_defaults() {
        let cfg = WsConfig::default();
        assert_eq!(cfg.bind.port(), 9222);
    }
}
