use super::*;
use crate::db::ProjectDb;
use crate::pipeline::{Pipeline, PipelineConfig};
use base64::Engine;
use std::sync::Arc;
use tokio::net::TcpListener;

fn test_server() -> (ApiServer, Arc<Pipeline>) {
    let db = ProjectDb::open_in_memory().unwrap();
    let dir = std::env::temp_dir().join(format!("jsb-api-{}", std::process::id()));
    let pipeline = Arc::new(Pipeline::new(
        db,
        PipelineConfig {
            workdir: dir,
            ..PipelineConfig::default()
        },
    ));
    let server = ApiServer::new(ApiConfig::default(), pipeline.clone());
    (server, pipeline)
}

#[test]
fn ingest_body_decodes_base64() {
    let b = IngestBody {
        url: "https://x/a.js".into(),
        content_type: "application/javascript".into(),
        content: base64::engine::general_purpose::STANDARD.encode("var x=1;"),
        encoding: Some("base64".into()),
        source_map_content: None,
        source: None,
    };
    assert_eq!(b.decode_content().unwrap(), b"var x=1;");
}

#[tokio::test]
async fn end_to_end_ingest_over_http() {
    let (server, pipeline) = test_server();

    // Bind on an ephemeral port and spawn the server.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let cfg = ApiConfig {
        bind: addr,
        ..ApiConfig::default()
    };
    let server = ApiServer {
        cfg: Arc::new(cfg),
        pipeline: server.pipeline.clone(),
    };
    tokio::spawn(server.serve());
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    // POST a JS asset with a secret to /ingest.
    let body = serde_json::json!({
        "url": "https://victim.test/app.js",
        "content_type": "application/javascript",
        "content": "const SECRET_KEY=\"ghp_abc123def456ghi789jkl012mno345pqr6\";fetch('/api/v2/users');",
    })
    .to_string();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/ingest"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(
        json["stored"].as_bool().unwrap_or(false),
        "expected stored=true: {json}"
    );
    assert!(json["findings"].as_u64().unwrap_or(0) > 0, "expected findings: {json}");

    // The pipeline persisted the file.
    let cur = crate::db::files::current_js_file(pipeline.db(), "https://victim.test/app.js").unwrap();
    assert!(cur.is_some());

    // /health works.
    let h = client.get(format!("http://{addr}/health")).send().await.unwrap();
    assert_eq!(h.status(), 200);
}

#[tokio::test]
async fn options_preflight_has_cors() {
    let (server, _p) = test_server();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let server = ApiServer {
        cfg: Arc::new(ApiConfig {
            bind: addr,
            ..ApiConfig::default()
        }),
        pipeline: server.pipeline.clone(),
    };
    tokio::spawn(server.serve());
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    let client = reqwest::Client::new();
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("http://{addr}/ingest"))
        .send()
        .await
        .unwrap();
    assert!(resp.headers().contains_key("access-control-allow-origin"));
}
