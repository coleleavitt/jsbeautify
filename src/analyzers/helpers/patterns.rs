use regex::Regex;
use std::sync::LazyLock;

macro_rules! lazy_regex {
    ($name:ident, $pat:expr) => {
        pub static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pat).expect(concat!("Failed to compile ", stringify!($name))));
    };
}

// ── Noise-filter regexes (a candidate matching ANY of these is dropped) ───────
lazy_regex!(START_PATTERN, r"^[a-zA-Z/]");
lazy_regex!(MODULE_IMPORT, r"^\.\.?/");
lazy_regex!(LOCALE_FILE, r"^[a-z]{2}(-[a-z]{2})?\.js$");
lazy_regex!(LOCALE_CODE, r"(?i)^[a-z]{2}(-[a-z]{2})?$");
lazy_regex!(SHA_PATTERN, r"^sha\d*$");
lazy_regex!(CRYPTO_MODULE, r"^aes$|^des$|^md5$");
lazy_regex!(PDF_STRUCTURE_1, r"^/[A-Z][a-z]+\s");
lazy_regex!(PDF_STRUCTURE_2, r"^/[A-Z][a-z]+$");
lazy_regex!(PDF_OBJECT_REF, r"^\d+ \d+ R$");
pub const EXCEL_XFORM_SUFFIX: &str = "-xform";
pub const EXCEL_INTERNAL_SUFFIX: &str = ".xml";
pub const DOC_PROPS_PREFIX: &str = "docProps/";
pub const RELATIONSHIPS_PREFIX: &str = "_rels/";
pub const META_INF_PREFIX: &str = "META-INF/";
pub const WORKSHEETS_PREFIX: &str = "worksheets/";
pub const THEME_DIR_PREFIX: &str = "theme/";
pub const WEBPACK_INTERNAL_PREFIX: &str = "webpack";
pub const ZONE_JS: &str = "zone.js";
pub const READABLE_STREAM_PREFIX: &str = "readable-stream/";
pub const PROCESS_DIR_PREFIX: &str = "process/";
pub const STREAM_DIR_PREFIX: &str = "stream/";
pub const BUFFER_MOD: &str = "buffer";
pub const EVENTS_MOD: &str = "events";
pub const UTIL_MOD: &str = "util";
pub const PATH_MOD: &str = "path";
lazy_regex!(SINGLE_LETTER_PATH_1, r"^/[a-z]$");
lazy_regex!(SINGLE_LETTER_PATH_2, r"^/[A-Z]$");
pub const EMPTY_HTTP: &str = "http://";
pub const NG_CONTENT: &str = "_ngcontent";
lazy_regex!(
    TIMEZONE_PATTERN,
    r"^(America|Europe|Asia|Africa|Australia|Pacific|Atlantic|Indian|Arctic|Antarctica)/[A-Za-z_]+"
);
pub const REF_QUERY_PREFIX: &str = "?ref=";
lazy_regex!(DOT_SLASH_PATTERN, r"^\.\.?/?$");
lazy_regex!(END_SLASH_PATTERN, r"^[^/]*/$");

// Additional patterns recovered from binary strings (REPORT-style, not surfaced
// in the markdown reports). Each one filters a class of false positives the
// original analyzer drops.
lazy_regex!(LONG_NUMBER_SUFFIX, r"^[a-zA-Z]+\d{4,}$");
lazy_regex!(LETTER_HEX_SUFFIX, r"^[a-zA-Z]{1,4}[0-9a-f]{10,}$");
lazy_regex!(LETTER_DIGITS, r"^[A-Za-z]\d+$");
lazy_regex!(UNDERSCORE_ONLY, r"^_+$");
lazy_regex!(CSS_CONCAT_PATTERN, r"\.concat[A-Za-z]|--[a-z]");
lazy_regex!(
    CSS_SPLIT_PATTERN,
    r#"%22|%3[cCeE]|%28|%29|%20|["'<>()\s\[\]\{\}\$`,.;:!?@#^&*+=|\\~/]--+|__+|-"#
);
lazy_regex!(TEMPLATE_VAR_PATTERN, r"\{\{|\}\}|\$\{|\}");

// Cookie attribute fragments — bundled inline cookie strings like
// `;path=/;expires=Mon, 01 Jan 2030 00:00:00 GMT;secure;httponly` would
// otherwise surface as `path` matches.
pub const COOKIE_ATTR_PREFIXES: &[&str] = &[
    ";path=",
    ";expires=",
    ";samesite=",
    ";secure",
    ";httponly",
    ";max-age=",
    ";same-site=",
    ";domain=",
    ";priority=",
    ";partitioned",
];

// React/Redux internal sentinels — ship in production bundles and otherwise
// fire `secret_*` heuristics or path heuristics.
pub const REACT_INTERNAL_SENTINELS: &[&str] = &[
    "__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED",
    "SECRET_DO_NOT_PASS_THIS_OR_YOU_WILL_BE_FIRED",
    "__REDUX_DEVTOOLS_EXTENSION__",
    "__REACT_DEVTOOLS_GLOBAL_HOOK__",
    "__REACT_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED",
    "__webpack_require__",
    "__webpack_modules__",
    "__webpack_exports__",
    "__esModule",
];

// String-literal noise tokens (substring matches)
pub const NOISE_LITERAL_PREFIXES: &[&str] = &[
    "./lib",
    "../lib",
    "./utils",
    "./node_modules",
    "../node_modules",
    "./src",
    "../src",
    "./dist",
    "../dist",
];

// Bare module names (full-string equality against candidate).
// Mirrors REPORT_03 §64-98 plus common third-party junk that surfaces in bundled output.
pub const BARE_MODULE_NAMES: &[&str] = &[
    "zone.js",
    "bn.js",
    "hash.js",
    "md5.js",
    "sha.js",
    "des.js",
    "asn1.js",
    "declare.js",
    "elliptic.js",
    "core-js",
    "regenerator-runtime",
    "tslib",
    "react-dom",
    "scheduler",
];

// ── Hostname / URL / Path / GraphQL extraction patterns ───────────────────────
lazy_regex!(
    HOSTNAME_PATTERN,
    r"(?i)(?:https?://|//)?([a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+)"
);
lazy_regex!(URL_PATTERN, r#"(?i)(https?://[^\s\)\]\}<>"']+|//[^\s\)\]\}<>"']+)"#);
lazy_regex!(PATH_PATTERN, r#"([a-zA-Z0-9/][^\s\)\]\}<>"']*(?:/[^\s\)\]\}<>"']*)+)"#);
lazy_regex!(GRAPHQL_REGEX, r"(query|mutation|fragment)");
lazy_regex!(
    PACKAGE_NAME_REGEX,
    r"^(@[a-z0-9-~][a-z0-9-._~]*/)?[a-z0-9-~][a-z0-9-._~]*$"
);
lazy_regex!(HTML_COMMENT, r"<!--([\s\S]*?)-->");

// Ignored hostnames (well-known third-party/standards domains we don't care about)
pub const NOISE_HOSTNAMES: &[&str] = &[
    "www.w3.org",
    "reactjs.org",
    "schemas.openxmlformats.org",
    "schemas.microsoft.com",
    "purl.oclc.org",
    "openoffice.org",
    "docs.oasis-open.org",
    "sheetjs.openxmlformats.org",
    "ns.adobe.com",
    "www.xml.org",
    "example.com",
    "jspdf.default.namespaceuri",
    "npmjs.org",
    "registry.npmjs.org",
    "github.com/indutny",
    "github.com/crypto-browserify",
    "jqwidgets.com",
    "ag-grid.com",
    "purl.org",
    "test.com",
    "randombase.com",
    "fusioncharts.com",
    // standard schemas / docs surfaces
    "www.iana.org",
    "tools.ietf.org",
    "datatracker.ietf.org",
    "schema.org",
    "schemas.xmlsoap.org",
    "json-schema.org",
    // CDN / build artifact hosts that aren't usually security-relevant
    "unpkg.com",
    "cdn.jsdelivr.net",
    "cdnjs.cloudflare.com",
    "registry.yarnpkg.com",
    // standards bodies
    "www.ecma-international.org",
    "tc39.es",
];
