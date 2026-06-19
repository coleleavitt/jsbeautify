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
