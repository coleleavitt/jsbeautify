//! Redux / Redux Toolkit (RTK) / RTK Query recon analyzer.
//!
//! Detects Redux-family state-management signatures that survive minification in
//! production bundles; emits one [`AnalyzerMatch`] per hit. Each detector maps to
//! a distinct snake_case `match_kind` prefixed with the framework.
//!
//! Signatures confirmed against the cloned current source:
//!   - `@@redux/INIT|REPLACE|PROBE_UNKNOWN_ACTION`  ->
//!       redux/src/utils/actionTypes.ts (lines 11-15)
//!   - `https://redux.js.org/Errors?code=`         ->
//!       redux/src/utils/formatProdErrorMessage.ts (line 10)
//!   - `Symbol.for("rtk-slice-createasyncthunk")`  ->
//!       redux-toolkit/packages/toolkit/src/createSlice.ts (lines 35-36)
//!   - `Symbol.for("rtk-state-proxy-original")`    ->
//!       redux-toolkit/packages/toolkit/src/combineSlices.ts (line 353)
//!   - `Symbol.for("RTKQ/skipToken")`              ->
//!       redux-toolkit/packages/toolkit/src/query/core/buildSelectors.ts (line 62)
//!   - `createSlice({name,reducers})` + `${slice}/${actionKey}` action types ->
//!       redux-toolkit/packages/toolkit/src/createSlice.ts (lines 217, 559)
//!   - `createAsyncThunk("prefix")` + `/pending|/fulfilled|/rejected`         ->
//!       redux-toolkit/packages/toolkit/src/createAsyncThunk.ts (lines 526-577)
//!   - `fetchBaseQuery({baseUrl})`, `reducerPath`, `tagTypes`, `build.query`  ->
//!       redux-toolkit/packages/toolkit/src/query/fetchBaseQuery.ts (line 201),
//!       redux-toolkit/packages/toolkit/src/query/createApi.ts (lines 99,68,126)

use super::match_kind::{AnalyzerMatch, FileType, PositionRange, Provenance};
use super::regex_bank::Severity;
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    // Redux core reserved action types + the prod-error doc URL.
    static ref CORE_RE: Regex = Regex::new(
        r#"(@@redux/(?:INIT|REPLACE|PROBE_UNKNOWN_ACTION)|redux\.js\.org/Errors\?code=)"#
    ).unwrap();

    // RTK internal Symbol.for markers (immutable across minification).
    static ref MARKER_RE: Regex = Regex::new(
        r#"Symbol\.for\(\s*["'](rtk-slice-createasyncthunk|rtk-state-proxy-original|RTKQ/skipToken)["']\s*\)"#
    ).unwrap();

    // createSlice({...name:"x"...}) — also matches minified webpack interop
    // form `(0,n.createSlice)({...})` via the optional `)` before the call paren.
    static ref SLICE_RE: Regex = Regex::new(
        r#"(?s)\bcreateSlice\)?\s*\(\s*\{.{0,400}?name\s*:\s*["']([\w$]+)["']"#
    ).unwrap();

    // createAsyncThunk("users/fetchById", ...) including `(0,r.createAsyncThunk)(...)`.
    static ref THUNK_RE: Regex = Regex::new(
        r#"\bcreateAsyncThunk\)?\s*\(\s*["']([^"']+)["']"#
    ).unwrap();

    // fetchBaseQuery({ ... baseUrl: "..." ... }) — RTK Query base query.
    static ref BASE_QUERY_RE: Regex = Regex::new(
        r#"(?s)\bfetchBaseQuery\)?\s*\(\s*\{.{0,300}?baseUrl\s*:\s*["']([^"']*)["']"#
    ).unwrap();

    // Per-endpoint definitions: `getPosts: build.query(` / `addPost: build.mutation(`.
    static ref ENDPOINT_RE: Regex = Regex::new(
        r#"([\w$]+)\s*:\s*build\.(query|mutation)\s*\("#
    ).unwrap();

    // createApi config fields, scanned once for metadata enrichment.
    static ref REDUCER_PATH_RE: Regex = Regex::new(
        r#"reducerPath\s*:\s*["']([\w$]+)["']"#
    ).unwrap();
    static ref TAG_TYPES_RE: Regex = Regex::new(
        r#"tagTypes\s*:\s*\[([^\]]*)\]"#
    ).unwrap();
    static ref STRING_LIT_RE: Regex = Regex::new(
        r#"["']([^"']+)["']"#
    ).unwrap();
}

/// Scan `source` for Redux / RTK / RTK Query signatures.
#[must_use]
pub fn analyze(source: &str, file_path: Option<&str>, file_type: FileType) -> Vec<AnalyzerMatch> {
    let mut out = Vec::new();
    let fp = || file_path.map(str::to_string);

    // --- redux_core: reserved action types + prod-error URL ---------------
    for caps in CORE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let val = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "redux_core",
                val,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // --- rtk_marker: internal Symbol.for() tags ---------------------------
    for caps in MARKER_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let val = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "rtk_marker",
                val,
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info),
        );
    }

    // --- rtk_slice: createSlice({name,reducers}) --------------------------
    // Action types generated by RTK follow `${slice}/${reducerKey}`
    // (createSlice.ts line 559), so we surface the slice name + action prefix.
    for caps in SLICE_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "rtk_slice",
                name.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "slice": name,
                "action_prefix": format!("{name}/"),
            })),
        );
    }

    // --- rtk_async_thunk: createAsyncThunk("prefix") ----------------------
    for caps in THUNK_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let prefix = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "rtk_async_thunk",
                prefix.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "type_prefix": prefix,
                "action_types": [
                    format!("{prefix}/pending"),
                    format!("{prefix}/fulfilled"),
                    format!("{prefix}/rejected"),
                ],
            })),
        );
    }

    // --- shared RTK Query createApi metadata (scanned once) ---------------
    let reducer_path = REDUCER_PATH_RE
        .captures(source)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let tags: Vec<String> = TAG_TYPES_RE
        .captures(source)
        .and_then(|c| c.get(1))
        .map(|m| {
            STRING_LIT_RE
                .captures_iter(m.as_str())
                .filter_map(|c| c.get(1))
                .map(|x| x.as_str().to_string())
                .collect()
        })
        .unwrap_or_default();
    let base_url = BASE_QUERY_RE
        .captures(source)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    // --- rtk_query_api: fetchBaseQuery({baseUrl}) -------------------------
    for caps in BASE_QUERY_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let url = caps.get(1).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "rtk_query_api",
                url.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "base_url": url,
                "reducer_path": reducer_path,
                "tags": tags,
            })),
        );
    }

    // --- rtk_query_endpoint: build.query / build.mutation -----------------
    for caps in ENDPOINT_RE.captures_iter(source) {
        let whole = caps.get(0).unwrap();
        let name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let kind = caps.get(2).map_or("", |m| m.as_str()).to_string();
        out.push(
            AnalyzerMatch::new(
                "rtk_query_endpoint",
                name.clone(),
                PositionRange::from_byte_range(source, whole.start(), whole.end()),
                file_type,
                fp(),
            )
            .with_provenance(Provenance::Other)
            .with_severity(Severity::Info)
            .with_metadata(serde_json::json!({
                "endpoint": name,
                "endpoint_type": kind,
                "base_url": base_url,
                "reducer_path": reducer_path,
                "tags": tags,
            })),
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_core_signature() {
        // Shape from redux/src/utils/actionTypes.ts after bundling/minification.
        let src = r#"var ActionTypes={INIT:"@@redux/INIT"+r(),REPLACE:"@@redux/REPLACE"+r(),PROBE_UNKNOWN_ACTION:function(){return"@@redux/PROBE_UNKNOWN_ACTION"+r()}};function e(n){return"Minified Redux error #"+n+"; visit https://redux.js.org/Errors?code="+n}"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(m.iter().any(|x| x.match_kind == "redux_core"));
        // Both @@redux/* tokens and the error URL should be found.
        assert!(m.iter().filter(|x| x.match_kind == "redux_core").count() >= 4);
    }

    #[test]
    fn detects_slice_thunk_and_marker() {
        // Minified webpack interop form: (0,n.createSlice)({...}).
        let src = r#"var s=(0,n.createSlice)({name:"counter",initialState:{value:0},reducers:{increment:function(e){e.value++}}});var f=(0,r.createAsyncThunk)("users/fetchById",function(e){return e});var k=Symbol.for("rtk-slice-createasyncthunk");"#;
        let m = analyze(src, Some("t.js"), FileType::Js);
        assert!(
            m.iter()
                .any(|x| x.match_kind == "rtk_slice" && x.match_value == "counter")
        );
        assert!(
            m.iter()
                .any(|x| x.match_kind == "rtk_async_thunk" && x.match_value == "users/fetchById")
        );
        assert!(
            m.iter()
                .any(|x| x.match_kind == "rtk_marker" && x.match_value == "rtk-slice-createasyncthunk")
        );
    }

    #[test]
    fn detects_rtk_query_api_and_endpoints() {
        let src = r#"var api=(0,t.createApi)({reducerPath:"api",baseQuery:(0,t.fetchBaseQuery)({baseUrl:"https://api.example.com/v1/"}),tagTypes:["Post","User"],endpoints:function(build){return{getPosts:build.query({query:function(){return"posts"}}),addPost:build.mutation({query:function(e){return{url:"posts",method:"POST",body:e}}})}}});"#;
        let m = analyze(src, Some("t.js"), FileType::Js);

        let api = m
            .iter()
            .find(|x| x.match_kind == "rtk_query_api")
            .expect("rtk_query_api match");
        assert_eq!(api.match_value, "https://api.example.com/v1/");
        let meta = api.metadata.as_ref().expect("metadata");
        assert_eq!(meta["reducer_path"], "api");
        assert_eq!(meta["tags"][0], "Post");
        assert_eq!(meta["tags"][1], "User");

        assert!(
            m.iter()
                .any(|x| x.match_kind == "rtk_query_endpoint" && x.match_value == "getPosts")
        );
        assert!(
            m.iter()
                .any(|x| x.match_kind == "rtk_query_endpoint" && x.match_value == "addPost")
        );
    }
}
