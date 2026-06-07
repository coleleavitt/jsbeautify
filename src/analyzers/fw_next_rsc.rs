//! Next.js (app-router / RSC) recon analyzer. Surfaces React Server Component
//! flight-stream, server-action, env, webpack-runtime, and `__NEXT_DATA__`
//! signatures that survive minification in production bundles.
//!
//! Signatures confirmed against the cloned Next.js source
//! (`/home/cole/RustProjects/forks/frameworks/nextjs`):
//!   - `self.__next_f` flight push:
//!       packages/next/src/server/app-render/use-flight-response.tsx:227
//!       packages/next/src/server/app-render/stream-ops.node.ts:910
//!       packages/next/src/client/app-index.tsx:174
//!   - flight `:I` import rows / `:HL` (H + "L") hint rows:
//!       packages/next/src/compiled/react-server-dom-webpack/cjs/
//!         react-server-dom-webpack-server.edge.production.js:1451 (`":I"`)
//!         react-server-dom-webpack-server.edge.production.js:1104 (`":H"` + code)
//!   - `createServerReference("<id>", ...)` server-action proxy:
//!       packages/next/src/build/webpack/loaders/
//!         next-flight-server-reference-proxy-loader.ts:24
//!   - `webpackChunk_N_E` runtime global (`library: '_N_E'`):
//!       packages/next/src/build/webpack-config.ts:1325
//!   - `__NEXT_DATA__` buildId/props:
//!       packages/next/src/client/index.tsx:206, :271
//!       packages/next/src/shared/lib/utils.ts:92 (`buildId: string`)
//!
//! Emits `AnalyzerMatch`. All regexes use only `regex`-crate-supported syntax
//! (no lookaround / backreferences).
use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// `self.__next_f.push([...])` or `(self.__next_f=self.__next_f||[]).push([...])`.
    /// Captures the leading bytes of the pushed payload.
    static ref FLIGHT_RE: Regex = Regex::new(
        r#"self\.__next_f(?:\s*=\s*self\.__next_f\s*\|\|\s*\[\s*\])?\s*\)?\s*\.push\(\s*\[([^\n\]]{0,80})"#
    ).unwrap();

    /// Flight stream rows: `<hexId>:I[...]` (module import) and `<hexId>:HL[...]`
    /// (preload hint, H-row with code `L`). Captures id, tag, payload prefix.
    static ref FLIGHT_ROW_RE: Regex =
        // No leading `\b`: flight rows are separated by the `\n` escape
        // (`\` + `n`), so the id is often preceded by a word char (`n`) and a
        // word boundary would never match the row after the first.
        Regex::new(r#"([0-9a-f]{1,8}):(HL|I)\[([^\n\]]{0,160})"#).unwrap();

    /// `createServerReference("<hexid>", ...)` server-action reference. Matches
    /// both the bare call and the `(0,n.createServerReference)(...)` member form.
    static ref SERVER_ACTION_RE: Regex =
        Regex::new(r#"(?:\(\s*0\s*,\s*[A-Za-z_$][0-9A-Za-z_$]*\.createServerReference\s*\)|createServerReference)\s*\(\s*"([0-9a-fA-F]{8,})""#).unwrap();

    /// `NEXT_PUBLIC_<NAME>` build-time inlined env identifier.
    static ref PUBLIC_ENV_RE: Regex = Regex::new(r#"\bNEXT_PUBLIC_([A-Z0-9_]+)"#).unwrap();

    /// `webpackChunk_N_E` webpack runtime global (the `_N_E` chunk-loading library).
    static ref WEBPACK_CHUNK_RE: Regex = Regex::new(r#"webpackChunk_N_E"#).unwrap();

    /// `__NEXT_DATA__` pages-router hydration payload (script id or window global).
    static ref NEXT_DATA_RE: Regex = Regex::new(r#"__NEXT_DATA__"#).unwrap();

    /// `"buildId":"<id>"` field inside the `__NEXT_DATA__` JSON payload.
    static ref BUILD_ID_RE: Regex = Regex::new(r#""buildId":"([^"]{1,200})""#).unwrap();
}

/// Scan source for Next.js app-router / RSC signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();

    // 1. Flight stream push.
    for caps in FLIGHT_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let payload = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
        out.push(
            AnalyzerMatch::new(
                "next_flight",
                payload,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // 2. Flight rows (module imports + preload hints).
    for caps in FLIGHT_ROW_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let row_id = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let tag = caps.get(2).map_or("", |m| m.as_str()).to_string();
        let payload = caps.get(3).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "next_flight_row",
                payload,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "row_id": row_id,
                "row_tag": tag,
            })),
        );
    }

    // 3. Server-action references (POST endpoint surface).
    for caps in SERVER_ACTION_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let action_id = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "next_server_action",
                action_id.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Medium)
            .with_metadata(serde_json::json!({
                "action_id": action_id,
                "surface": "server_action_post",
            })),
        );
    }

    // 4. NEXT_PUBLIC_ inlined env identifiers.
    for caps in PUBLIC_ENV_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = whole.as_str().to_string();
        out.push(
            AnalyzerMatch::new(
                "next_public_env",
                name.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Low)
            .with_metadata(serde_json::json!({ "env_var": name })),
        );
    }

    // 5. webpackChunk_N_E runtime global.
    for m in WEBPACK_CHUNK_RE.find_iter(source) {
        out.push(
            AnalyzerMatch::new(
                "next_webpack_chunk",
                m.as_str().to_string(),
                PositionRange::from_byte_range(source, m.start(), m.end()),
                file_type,
                file_path.map(str::to_string),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // 6. __NEXT_DATA__ hydration payload (attach buildId if present nearby).
    let build_id = BUILD_ID_RE
        .captures(source)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    for m in NEXT_DATA_RE.find_iter(source) {
        let mut am = AnalyzerMatch::new(
            "next_data",
            "__NEXT_DATA__",
            PositionRange::from_byte_range(source, m.start(), m.end()),
            file_type,
            file_path.map(str::to_string),
        )
        .with_provenance(Provenance::Other)
        .with_severity(Severity::Info);
        if let Some(bid) = &build_id {
            am = am.with_metadata(serde_json::json!({ "build_id": bid }));
        }
        out.push(am);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Minified-shape flight stream: a __next_f push whose payload string
        // carries an `:I` module-import row and an `:HL` preload-hint row.
        let src = r#"(self.__next_f=self.__next_f||[]).push([1,"3:I[\"4567\",[\"static/chunks/app-9f2.js\"],\"default\"]\n2:HL[\"/_next/static/css/x.css\",\"style\"]\n"])"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "next_flight"));
        let rows: Vec<_> = m.iter().filter(|x| x.match_kind == "next_flight_row").collect();
        assert!(rows.iter().any(|x| {
            x.metadata
                .as_ref()
                .and_then(|md| md.get("row_tag"))
                .and_then(|v| v.as_str())
                == Some("I")
        }));
        assert!(rows.iter().any(|x| {
            x.metadata
                .as_ref()
                .and_then(|md| md.get("row_tag"))
                .and_then(|v| v.as_str())
                == Some("HL")
        }));
    }

    #[test]
    fn detects_server_action_env_and_chunk() {
        let src = r#"(self.webpackChunk_N_E=self.webpackChunk_N_E||[]).push([[123],{456:function(e,t,n){var a=n.createServerReference("7c0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c",callServer,void 0,findSourceMapURL,"submit");t.u=process.env.NEXT_PUBLIC_API_URL}}]);"#;
        let m = analyze(src, Some("chunk.js"), FileType::Js);

        let action = m
            .iter()
            .find(|x| x.match_kind == "next_server_action")
            .expect("server action");
        assert_eq!(action.match_value, "7c0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c");
        assert_eq!(action.severity, Severity::Medium);

        let env = m
            .iter()
            .find(|x| x.match_kind == "next_public_env")
            .expect("public env");
        assert_eq!(env.match_value, "NEXT_PUBLIC_API_URL");
        assert_eq!(env.severity, Severity::Low);

        assert!(m.iter().any(|x| x.match_kind == "next_webpack_chunk"));
    }

    #[test]
    fn detects_server_action_interop_member_call() {
        let src =
            r#"var a=(0,n.createServerReference)("abcdef1234567890",callServer,void 0,findSourceMapURL,"submit");"#;
        let m = analyze(src, Some("chunk.js"), FileType::Js);
        assert!(
            m.iter()
                .any(|x| x.match_kind == "next_server_action" && x.match_value == "abcdef1234567890"),
            "expected server action from webpack interop member call: {m:?}"
        );
    }

    #[test]
    fn detects_next_data_with_build_id() {
        let src = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{}},"page":"/","query":{},"buildId":"abc123XYZ"}</script>"#;
        let m = analyze(src, Some("index.html"), FileType::Html);
        let nd = m.iter().find(|x| x.match_kind == "next_data").expect("next_data");
        assert_eq!(
            nd.metadata
                .as_ref()
                .and_then(|md| md.get("build_id"))
                .and_then(|v| v.as_str()),
            Some("abc123XYZ")
        );
    }
}
