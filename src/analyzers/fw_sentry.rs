//! Sentry browser SDK recon analyzer. Fingerprints the @sentry/* runtime,
//! transport DSNs, and build-time markers left in production bundles.
//!
//! Signatures that survive minification in production bundles; emits AnalyzerMatch.
//!
//! All literals confirmed against the cloned upstream source at
//! /home/cole/RustProjects/forks/frameworks/sentry-js:
//!   - DSN host/org regex .................. packages/core/src/utils/dsn.ts
//!                                           (DSN_REGEX, ORG_ID_REGEX `^o(\d+)\.`)
//!   - ingest auth querystring ............. packages/core/src/api.ts
//!                                           (`sentry_version`, `sentry_key`),
//!                                           packages/core/src/utils/isSentryRequestUrl.ts
//!                                           (`/(^|&|\?)sentry_key=/`),
//!                                           packages/browser/src/diagnose-sdk.ts
//!                                           (`?sentry_version=7&sentry_key=...`)
//!   - __SENTRY__ carrier .................. packages/core/src/carrier.ts
//!   - sentry.javascript.<pkg> sdk name .... packages/core/src/utils/sdkMetadata.ts:21
//!                                           (`sdk.name = `sentry.javascript.${name}``),
//!                                           packages/browser/src/profiling/utils.ts:237
//!   - integration .name literals .......... packages/browser/src/integrations/breadcrumbs.ts:60,
//!                                           globalhandlers.ts:24, httpclient.ts:21,
//!                                           Replay (packages/core/src/logs/internal.ts:120)
//!   - _sentryDebugIds / _debugIds ......... packages/core/src/utils/debug-ids.ts:30-31,
//!                                           packages/core/src/utils/worldwide.ts:43-49
//!   - SENTRY_RELEASE/DSN/ENVIRONMENT ...... packages/core/src/utils/worldwide.ts:32-34
//!   - Spotlight localhost:8969 ............ packages/browser/src/integrations/spotlight.ts:20
//!                                           (`'http://localhost:8969/stream'`)

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use regex::Regex;

/// Scan source for Sentry browser SDK signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    lazy_static::lazy_static! {
        // Full DSN: https://<publicKey>@o<orgId>.ingest(.<region>)?.sentry.io/<projectId>
        // Anchored on the org host + .sentry.io, never a bare URL.
        static ref DSN_RE: Regex = Regex::new(
            r#"https://([0-9a-fA-F]{16,64})@(o\d+\.ingest(?:\.[a-z]{2,4})?\.sentry\.io)/(\d+)"#
        ).unwrap();
        // Ingest envelope endpoint auth querystring: ...?sentry_version=7&sentry_key=<key>
        static ref INGEST_AUTH_RE: Regex = Regex::new(
            r#"sentry_version=7&sentry_key=([0-9a-fA-F]{16,64})"#
        ).unwrap();
        // Global carrier object that every Sentry SDK pins to the host global.
        static ref CARRIER_RE: Regex = Regex::new(r#"__SENTRY__"#).unwrap();
        // SDK name template: sentry.javascript.<pkg> (browser, react, vue, ...).
        static ref SDK_NAME_RE: Regex = Regex::new(
            r#"sentry\.javascript\.([a-z][a-z0-9-]*)"#
        ).unwrap();
        // Integration `name` literal for high-signal default integrations.
        static ref INTEGRATION_RE: Regex = Regex::new(
            r#"name:["'](Breadcrumbs|GlobalHandlers|Replay|HttpClient)["']"#
        ).unwrap();
        // Debug-id maps injected by the bundler plugin (source map correlation).
        static ref DEBUG_ID_RE: Regex = Regex::new(r#"_sentryDebugIds|_debugIds"#).unwrap();
        // Build-time environment markers.
        static ref ENV_RE: Regex = Regex::new(r#"SENTRY_(?:RELEASE|DSN|ENVIRONMENT)\b"#).unwrap();
        // Spotlight (dev sidecar) default endpoint.
        static ref SPOTLIGHT_RE: Regex = Regex::new(r#"localhost:8969(?:/stream)?"#).unwrap();
    }

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
    for caps in CARRIER_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "sentry_global",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
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
    for caps in DEBUG_ID_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "sentry_debug_id",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
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
    for caps in SPOTLIGHT_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        out.push(
            AnalyzerMatch::new(
                "sentry_spotlight",
                whole.as_str(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
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
                .map(|md| md["host"].as_str() == Some("o447951.ingest.us.sentry.io"))
                .unwrap_or(false)
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
