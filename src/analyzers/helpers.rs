//! Shared helpers + noise-suppression patterns.
//!
//! All regexes recovered from jxscout-pro-v2 v2.1.1 binary
//! (`crates/core/analyzers/src/helpers.rs`).
//!
//! These are used by the path/hostname/URL analyzers to filter out common
//! false positives (timezone strings, locale codes, NPM relative imports,
//! Excel/PDF internals, …).

use lazy_static::lazy_static;
use regex::Regex;

macro_rules! lazy_regex {
    ($name:ident, $pat:expr) => {
        lazy_static! {
            pub static ref $name: Regex = Regex::new($pat).expect(concat!("Failed to compile ", stringify!($name)));
        }
    };
}

// ── Noise-filter regexes (a candidate matching ANY of these is dropped) ───────
lazy_regex!(START_PATTERN, r"^[a-zA-Z/]");
lazy_regex!(MODULE_IMPORT, r"^\.\.?/");
lazy_regex!(LOCALE_FILE, r"^[a-z]{2}(-[a-z]{2})?\.js$");
lazy_regex!(LOCALE_CODE, r"(?i)^[a-z]{2}(-[a-z]{2})?$");
lazy_regex!(EXCEL_XFORM, r"-xform$");
lazy_regex!(SHA_PATTERN, r"^sha\d*$");
lazy_regex!(CRYPTO_MODULE, r"^aes$|^des$|^md5$");
lazy_regex!(PDF_STRUCTURE_1, r"^/[A-Z][a-z]+\s");
lazy_regex!(PDF_STRUCTURE_2, r"^/[A-Z][a-z]+$");
lazy_regex!(PDF_OBJECT_REF, r"^\d+ \d+ R$");
lazy_regex!(EXCEL_INTERNAL, r"\.xml$");
lazy_regex!(DOC_PROPS, r"^docProps/");
lazy_regex!(RELATIONSHIPS, r"^_rels/");
lazy_regex!(META_INF, r"^META-INF/");
lazy_regex!(WORKSHEETS, r"^worksheets/");
lazy_regex!(THEME_DIR, r"^theme/");
lazy_regex!(WEBPACK_INTERNAL, r"^webpack");
lazy_regex!(ZONE_JS, r"^zone\.js$");
lazy_regex!(READABLE_STREAM, r"^readable-stream/");
lazy_regex!(PROCESS_DIR, r"^process/");
lazy_regex!(STREAM_DIR, r"^stream/");
lazy_regex!(BUFFER_MOD, r"^buffer$");
lazy_regex!(EVENTS_MOD, r"^events$");
lazy_regex!(UTIL_MOD, r"^util$");
lazy_regex!(PATH_MOD, r"^path$");
lazy_regex!(STARTS_PLUS, r"^\+");
lazy_regex!(STARTS_TEMPLATE, r"^\$\{");
lazy_regex!(STARTS_QUERY, r"^#");
lazy_regex!(SINGLE_LETTER_PATH_1, r"^/[a-z]$");
lazy_regex!(SINGLE_LETTER_PATH_2, r"^/[A-Z]$");
lazy_regex!(EMPTY_HTTP, r"^http://$");
lazy_regex!(NG_CONTENT, r"_ngcontent");
lazy_regex!(
    TIMEZONE_PATTERN,
    r"^(America|Europe|Asia|Africa|Australia|Pacific|Atlantic|Indian|Arctic|Antarctica)/[A-Za-z_]+"
);
lazy_regex!(REF_QUERY, r"^\?\ref=");
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

/// Return true if `value` looks like a path/hostname noise pattern.
#[must_use]
pub fn is_path_noise(value: &str) -> bool {
    // bare module names
    if BARE_MODULE_NAMES.contains(&value) {
        return true;
    }
    // React/webpack internal sentinels
    if REACT_INTERNAL_SENTINELS.contains(&value) {
        return true;
    }
    // Cookie attribute prefixes (case-insensitive substring at start)
    {
        let lower = value.to_ascii_lowercase();
        for p in COOKIE_ATTR_PREFIXES {
            if lower.starts_with(p) {
                return true;
            }
        }
    }
    // literal prefix matches
    for p in NOISE_LITERAL_PREFIXES {
        if value == *p || value.starts_with(*p) {
            return true;
        }
    }
    // regex match (ANY of these)
    [
        &*MODULE_IMPORT,
        &*LOCALE_FILE,
        &*LOCALE_CODE,
        &*EXCEL_XFORM,
        &*SHA_PATTERN,
        &*CRYPTO_MODULE,
        &*PDF_STRUCTURE_1,
        &*PDF_STRUCTURE_2,
        &*PDF_OBJECT_REF,
        &*EXCEL_INTERNAL,
        &*DOC_PROPS,
        &*RELATIONSHIPS,
        &*META_INF,
        &*WORKSHEETS,
        &*THEME_DIR,
        &*WEBPACK_INTERNAL,
        &*ZONE_JS,
        &*READABLE_STREAM,
        &*PROCESS_DIR,
        &*STREAM_DIR,
        &*BUFFER_MOD,
        &*EVENTS_MOD,
        &*UTIL_MOD,
        &*PATH_MOD,
        &*STARTS_PLUS,
        &*STARTS_TEMPLATE,
        &*STARTS_QUERY,
        &*SINGLE_LETTER_PATH_1,
        &*SINGLE_LETTER_PATH_2,
        &*EMPTY_HTTP,
        &*NG_CONTENT,
        &*TIMEZONE_PATTERN,
        &*REF_QUERY,
        &*DOT_SLASH_PATTERN,
        &*END_SLASH_PATTERN,
        &*LONG_NUMBER_SUFFIX,
        &*LETTER_HEX_SUFFIX,
        &*LETTER_DIGITS,
        &*UNDERSCORE_ONLY,
        &*CSS_CONCAT_PATTERN,
        &*TEMPLATE_VAR_PATTERN,
    ]
    .iter()
    .any(|r| r.is_match(value))
}

/// Return true if a string starts with at least one alphanumeric or `/`.
/// Used by analyzers to skip empty/junk candidates.
#[must_use]
pub fn passes_start_pattern(value: &str) -> bool {
    START_PATTERN.is_match(value)
}

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

#[must_use]
pub fn is_hostname_noise(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    NOISE_HOSTNAMES
        .iter()
        .any(|n| lower == *n || lower.ends_with(&format!(".{n}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_noise_filters() {
        assert!(is_path_noise("./lib/foo"));
        assert!(is_path_noise("America/New_York"));
        assert!(is_path_noise("en.js"));
        assert!(is_path_noise("en-US"));
        assert!(is_path_noise("zone.js"));
        assert!(is_path_noise("/A"));
        assert!(is_path_noise("META-INF/manifest"));
        assert!(is_path_noise("docProps/app.xml"));
        assert!(is_path_noise("../node_modules/foo"));

        assert!(!is_path_noise("/api/v2/users"));
        assert!(!is_path_noise("/admin/login"));
    }

    #[test]
    fn cookie_attrs_are_noise() {
        assert!(is_path_noise(";path=/"));
        assert!(is_path_noise(";Path=/api"));
        assert!(is_path_noise(";expires=Mon, 01 Jan 2030 00:00:00 GMT"));
        assert!(is_path_noise(";samesite=Strict"));
        assert!(is_path_noise(";secure"));
        assert!(is_path_noise(";HttpOnly"));
    }

    #[test]
    fn react_sentinels_are_noise() {
        assert!(is_path_noise("__SECRET_INTERNALS_DO_NOT_USE_OR_YOU_WILL_BE_FIRED"));
        assert!(is_path_noise("__REDUX_DEVTOOLS_EXTENSION__"));
        assert!(is_path_noise("__webpack_require__"));
    }

    #[test]
    fn long_number_suffix_is_noise() {
        // Mangled identifiers like `api1234567` (>= 4 trailing digits)
        assert!(is_path_noise("api12345"));
        assert!(is_path_noise("foo7777"));
        // Single-letter + digits is also noise (LETTER_DIGITS pattern)
        assert!(is_path_noise("v123"));
        assert!(is_path_noise("a1"));
        // Multi-letter with 1-3 trailing digits passes
        assert!(!is_path_noise("api2"));
        assert!(!is_path_noise("login"));
    }

    #[test]
    fn letter_hex_suffix_is_noise() {
        // Webpack content hash: `app1a2b3c4d5e6f` style
        assert!(is_path_noise("app1234567890ab"));
        assert!(is_path_noise("a1234567890abcdef"));
        // Normal hostname with hex-like chars passes
        assert!(!is_path_noise("api.example.com"));
    }

    #[test]
    fn underscore_only_is_noise() {
        assert!(is_path_noise("___"));
        assert!(is_path_noise("____"));
        assert!(!is_path_noise("__init__"));
    }

    #[test]
    fn hostname_noise() {
        assert!(is_hostname_noise("www.w3.org"));
        assert!(is_hostname_noise("schemas.openxmlformats.org"));
        assert!(is_hostname_noise("foo.example.com")); // suffix match
        assert!(!is_hostname_noise("api.target.com"));
    }

    #[test]
    fn package_name_regex_works() {
        assert!(PACKAGE_NAME_REGEX.is_match("react"));
        assert!(PACKAGE_NAME_REGEX.is_match("@scope/pkg"));
        assert!(PACKAGE_NAME_REGEX.is_match("lodash.merge"));
        assert!(!PACKAGE_NAME_REGEX.is_match("/api/v2"));
        assert!(!PACKAGE_NAME_REGEX.is_match("PascalCase"));
    }

    #[test]
    fn graphql_keyword_match() {
        assert!(GRAPHQL_REGEX.is_match("query GetUsers { ... }"));
        assert!(GRAPHQL_REGEX.is_match("mutation { ... }"));
        assert!(GRAPHQL_REGEX.is_match("fragment Foo on Bar { ... }"));
    }
}
