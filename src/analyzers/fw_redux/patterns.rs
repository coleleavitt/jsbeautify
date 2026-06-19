use regex::Regex;
use std::sync::LazyLock;

// Redux core reserved action types, prod-error doc URL, and the devtools
// extension sentinel (the strongest Redux/Zustand store signal; the path
// extractor scrubs it as noise, but this analyzer sees raw source).
pub(super) static CORE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(@@redux/(?:INIT|REPLACE|PROBE_UNKNOWN_ACTION)|redux\.js\.org/Errors\?code=|__REDUX_DEVTOOLS_EXTENSION__(?:_COMPOSE__)?)"
    ).unwrap()
});

// RTK internal Symbol.for markers (immutable across minification).
pub(super) static MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Symbol\.for\(\s*["'](rtk-slice-createasyncthunk|rtk-state-proxy-original|RTKQ/skipToken)["']\s*\)"#)
        .unwrap()
});

// createSlice({...name:"x"...}) — also matches minified webpack interop
// form `(0,n.createSlice)({...})` via the optional `)` before the call paren.
pub(super) static SLICE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)\bcreateSlice\)?\s*\(\s*\{.{0,400}?name\s*:\s*["']([\w$]+)["']"#).unwrap());

// createAsyncThunk("users/fetchById", ...) including `(0,r.createAsyncThunk)(...)`.
pub(super) static THUNK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bcreateAsyncThunk\)?\s*\(\s*["']([^"']+)["']"#).unwrap());

// fetchBaseQuery({ ... baseUrl: "..." ... }) — RTK Query base query.
pub(super) static BASE_QUERY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)\bfetchBaseQuery\)?\s*\(\s*\{.{0,300}?baseUrl\s*:\s*["']([^"']*)["']"#).unwrap()
});

// Per-endpoint definitions: `getPosts: build.query(` / `addPost: build.mutation(`.
// The builder parameter is mangled in production bundles (`e.query`/`b.query`),
// so it is captured (group 2) and validated against the real `endpoints:(x)=>`
// parameter name rather than hardcoding `build` — which would both miss
// minified code and, if blindly broadened, false-positive on any
// `key: obj.query(`. Group 1 = endpoint name, 2 = builder ident, 3 = kind.
pub(super) static ENDPOINT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([\w$]+)\s*:\s*([\w$]+)\.(query|mutation)\s*\(").unwrap());

// The builder parameter name of `endpoints: (build) => ({...})`. `endpoints`
// is a fixed RTK Query key (survives minification); only its parameter is
// mangled, so capturing it here lets ENDPOINT_RE bind to the real builder.
// The trailing `)`/`=>` requirement ensures the captured token is an actual
// callback parameter, not a bare `endpoints: someRef` value.
pub(super) static BUILDER_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"endpoints\s*:\s*(?:async\b\s*)?(?:function\s*\*?\s*)?\(?\s*([\w$]+)\s*(?:\)|=>)").unwrap()
});

// createApi config fields, scanned once for metadata enrichment.
pub(super) static REDUCER_PATH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"reducerPath\s*:\s*["']([\w$]+)["']"#).unwrap());

pub(super) static TAG_TYPES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"tagTypes\s*:\s*\[([^\]]*)\]").unwrap());

pub(super) static STRING_LIT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["']([^"']+)["']"#).unwrap());
