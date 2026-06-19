//! Sentry browser SDK recon analyzer. Fingerprints the @sentry/* runtime,
//! transport DSNs, and build-time markers left in production bundles.
//!
//! Signatures that survive minification in production bundles; emits `AnalyzerMatch`.
//!
//! All literals confirmed against the cloned upstream source at
//! `/home/cole/RustProjects/forks/frameworks/sentry-js`:
//!
//! - DSN host/org regex -> packages/core/src/utils/dsn.ts
//! - ingest auth querystring -> packages/core/src/api.ts
//! - `__SENTRY__` carrier -> packages/core/src/carrier.ts
//! - `sentry.javascript.<pkg>` SDK name -> packages/core/src/utils/sdkMetadata.ts:21
//! - integration `.name` literals -> packages/browser/src/integrations
//! - `_sentryDebugIds` / `_debugIds` -> packages/core/src/utils/debug-ids.ts:30-31
//! - `SENTRY_RELEASE` / `SENTRY_DSN` / `SENTRY_ENVIRONMENT` -> packages/core/src/utils/worldwide.ts
//! - Spotlight `localhost:8969` -> packages/browser/src/integrations/spotlight.ts:20

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;
use std::sync::LazyLock;

static DSN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://([0-9a-fA-F]{16,64})@(o\d+\.ingest(?:\.[a-z]{2,4})?\.sentry\.io)/(\d+)").unwrap()
});
static INGEST_AUTH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sentry_version=7&sentry_key=([0-9a-fA-F]{16,64})").unwrap());
static SDK_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"sentry\.javascript\.([a-z][a-z0-9-]*)").unwrap());
static INTEGRATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"name:["'](Breadcrumbs|GlobalHandlers|Replay|HttpClient)["']"#).unwrap());
static ENV_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"SENTRY_(?:RELEASE|DSN|ENVIRONMENT)\b").unwrap());

const SENTRY_CARRIER: &str = "__SENTRY__";
const DEBUG_ID_MARKERS: [&str; 2] = ["_sentryDebugIds", "_debugIds"];
const SPOTLIGHT_HOST: &str = "localhost:8969";
const SPOTLIGHT_STREAM: &str = "localhost:8969/stream";

/// Scan source for Sentry browser SDK signatures.
///
/// # Panics
///
/// Panics only if a compile-time regex pattern is invalid.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();

    // sentry_dsn — full DSN (Medium): leaks the project's public ingest key + org/project ids.
    for caps in DSN_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let key = caps.get(1).map_or("", |m| m.as_str());
        let host = caps.get(2).map_or("", |m| m.as_str());
        let project = caps.get(3).map_or("", |m| m.as_str());
        out.push(
            AnalyzerMatch::new(
                "sentry_dsn",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Medium)
            .with_metadata(serde_json::json!({
                "public_key": key,
                "host": host,
                "project_id": project,
                "form": "dsn",
            })),
        );
    }

    // sentry_dsn — ingest envelope auth querystring (Medium): same public key, transport form.
    for caps in INGEST_AUTH_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let key = caps.get(1).map_or("", |m| m.as_str());
        out.push(
            AnalyzerMatch::new(
                "sentry_dsn",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Medium)
            .with_metadata(serde_json::json!({
                "public_key": key,
                "sentry_version": "7",
                "form": "ingest_auth",
            })),
        );
    }

    // sentry_global — the __SENTRY__ carrier.
    for (start, value) in source.match_indices(SENTRY_CARRIER) {
        out.push(
            AnalyzerMatch::new(
                "sentry_global",
                value,
                PositionRange::from_byte_range(source, start, start + value.len()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // sentry_sdk — sentry.javascript.<pkg> name, reveals which SDK flavor shipped.
    for caps in SDK_NAME_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let pkg = caps.get(1).map_or("", |m| m.as_str());
        out.push(
            AnalyzerMatch::new(
                "sentry_sdk",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({ "package": pkg })),
        );
    }

    // sentry_integration — default integration name literals.
    for caps in INTEGRATION_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str());
        out.push(
            AnalyzerMatch::new(
                "sentry_integration",
                name,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // sentry_debug_id — debug-id map globals injected by the bundler plugin.
    for marker in DEBUG_ID_MARKERS {
        for (start, value) in source.match_indices(marker) {
            out.push(
                AnalyzerMatch::new(
                    "sentry_debug_id",
                    value,
                    PositionRange::from_byte_range(source, start, start + value.len()),
                    file_type,
                    file_path.map(str::to_string),
                )
                .with_provenance(Provenance::Other)
                .with_severity(Severity::Info),
            );
        }
    }

    // sentry_env — build-time SENTRY_* environment markers.
    for caps in ENV_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "sentry_env",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // sentry_spotlight — local Spotlight sidecar endpoint left in a shipped bundle.
    for (start, _) in source.match_indices(SPOTLIGHT_HOST) {
        let value = if source[start..].starts_with(SPOTLIGHT_STREAM) {
            SPOTLIGHT_STREAM
        } else {
            SPOTLIGHT_HOST
        };
        out.push(
            AnalyzerMatch::new(
                "sentry_spotlight",
                value,
                PositionRange::from_byte_range(source, start, start + value.len()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Minified-shape carrier + sdk name, as emitted by @sentry/browser.
        let src = r#"var t=e.__SENTRY__=e.__SENTRY__||{};t.sdk={name:"sentry.javascript.browser",version:"8.0.0"};"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "sentry_global"));
        let sdk = m.iter().find(|x| x.match_kind == "sentry_sdk").unwrap();
        assert_eq!(sdk.metadata.as_ref().unwrap()["package"].as_str(), Some("browser"));
    }

    #[test]
    fn detects_dsn_and_ingest_auth() {
        // Real DSN shape (from diagnose-sdk.ts) + the ingest envelope auth querystring.
        let src = r#"Sentry.init({dsn:"https://c1dfb07d783ad5325c245c1fd3725390@o447951.ingest.us.sentry.io/4509632503087104"});var u="https://o447951.ingest.sentry.io/api/4509632503087104/envelope/?sentry_version=7&sentry_key=c1dfb07d783ad5325c245c1fd3725390&sentry_client=sentry.javascript.browser%2F1.33.7";"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        let dsns: Vec<_> = m.iter().filter(|x| x.match_kind == "sentry_dsn").collect();
        assert!(dsns.len() >= 2, "expected DSN + ingest-auth forms");
        assert!(dsns.iter().all(|x| x.severity == Severity::Medium));
        // The full-DSN form carries org host + project id metadata.
        assert!(dsns.iter().any(|x| {
            x.metadata
                .as_ref()
                .is_some_and(|md| md["host"].as_str() == Some("o447951.ingest.us.sentry.io"))
        }));
    }

    #[test]
    fn detects_integrations_debug_ids_env_spotlight() {
        let src = r#"[{name:"GlobalHandlers"},{name:'Breadcrumbs'},{name:"Replay"},{name:"HttpClient"}];var d=GLOBAL_OBJ._sentryDebugIds||GLOBAL_OBJ._debugIds;var r=process.env.SENTRY_RELEASE,e=process.env.SENTRY_ENVIRONMENT;makeFetch("http://localhost:8969/stream");"#;
        let m = analyze(src, None, FileType::Js);
        let kinds: Vec<&str> = m.iter().map(|x| x.match_kind.as_str()).collect();
        assert!(kinds.contains(&"sentry_integration"));
        assert!(kinds.contains(&"sentry_debug_id"));
        assert!(kinds.contains(&"sentry_env"));
        assert!(kinds.contains(&"sentry_spotlight"));
        // All four default integration names captured.
        let names: Vec<&str> = m
            .iter()
            .filter(|x| x.match_kind == "sentry_integration")
            .map(|x| x.match_value.as_str())
            .collect();
        assert!(names.contains(&"GlobalHandlers") && names.contains(&"Replay"));
    }
}
