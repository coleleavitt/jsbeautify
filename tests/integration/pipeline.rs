//! Pipeline ingest integration tests: verify on-disk artifacts are produced.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const THREE_MIB: usize = 3 * 1024 * 1024;

use js_beautify_rs::db::ProjectDb;
use js_beautify_rs::db::files::IngestionSource;
use js_beautify_rs::pipeline::{IngestRequest, Pipeline, PipelineConfig};

use crate::fixtures;

fn unique_workdir() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("jsb-itest-{}-{n}", std::process::id()))
}

fn test_pipeline(workdir: PathBuf) -> Pipeline {
    let db = ProjectDb::open_in_memory().expect("in-memory db");
    Pipeline::new(
        db,
        PipelineConfig {
            workdir,
            ..PipelineConfig::default()
        },
    )
}

fn js_request(url: &str, body: &str) -> IngestRequest {
    IngestRequest {
        url: url.to_string(),
        content_type: "application/javascript".to_string(),
        content: body.as_bytes().to_vec(),
        source: IngestionSource::Cli,
        source_map_content: None,
        request_headers: None,
    }
}

#[test]
fn ingest_writes_raw_and_beautified() {
    let workdir = unique_workdir();
    let pipeline = test_pipeline(workdir.clone());
    let src = fixtures::obfuscated("simple_string_array.js");

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    let result = rt.block_on(pipeline.ingest(js_request("https://victim.test/app.js", &src)));

    assert!(result.stored, "asset should be stored: {result:?}");
    assert!(result.error.is_none(), "no ingest error: {result:?}");

    let raw = workdir.join("raw").join("victim.test").join("app.js");
    let pretty = workdir.join("beautified").join("victim.test").join("app.js");
    assert!(raw.exists(), "raw file should be written at {}", raw.display());
    assert!(
        pretty.exists(),
        "beautified file should be written at {}",
        pretty.display()
    );

    std::fs::remove_dir_all(&workdir).ok();
}

#[test]
fn large_input_is_still_processed() {
    // Regression for the removed 24 MiB cap: a multi-megabyte file must still
    // be decoded, beautified, and written rather than silently skipped.
    let workdir = unique_workdir();
    let pipeline = test_pipeline(workdir.clone());

    // ~3 MiB of valid, minified-ish JS.
    let mut big = String::with_capacity(THREE_MIB);
    let mut n = 0u32;
    while big.len() < THREE_MIB {
        write!(big, "var v{n}={{a:{n},b:{n}}};").expect("write to String");
        n += 1;
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    let result = rt.block_on(pipeline.ingest(js_request("https://victim.test/big.js", &big)));

    assert!(result.stored);
    assert!(
        result.beautified_bytes > 0,
        "large file should be beautified, got {result:?}"
    );
    let pretty = workdir.join("beautified").join("victim.test").join("big.js");
    assert!(pretty.exists(), "beautified large file should exist");

    std::fs::remove_dir_all(&workdir).ok();
}
