use super::patterns::{
    BARE_MODULE_NAMES, BUFFER_MOD, COOKIE_ATTR_PREFIXES, CRYPTO_MODULE, CSS_CONCAT_PATTERN, DOC_PROPS_PREFIX,
    DOT_SLASH_PATTERN, EMPTY_HTTP, END_SLASH_PATTERN, EVENTS_MOD, EXCEL_INTERNAL_SUFFIX, EXCEL_XFORM_SUFFIX,
    LETTER_DIGITS, LETTER_HEX_SUFFIX, LOCALE_CODE, LOCALE_FILE, LONG_NUMBER_SUFFIX, META_INF_PREFIX, MODULE_IMPORT,
    NG_CONTENT, NOISE_HOSTNAMES, NOISE_LITERAL_PREFIXES, PATH_MOD, PDF_OBJECT_REF, PDF_STRUCTURE_1, PDF_STRUCTURE_2,
    PROCESS_DIR_PREFIX, REACT_INTERNAL_SENTINELS, READABLE_STREAM_PREFIX, REF_QUERY_PREFIX, RELATIONSHIPS_PREFIX,
    SHA_PATTERN, SINGLE_LETTER_PATH_1, SINGLE_LETTER_PATH_2, START_PATTERN, STREAM_DIR_PREFIX, TEMPLATE_VAR_PATTERN,
    THEME_DIR_PREFIX, TIMEZONE_PATTERN, UNDERSCORE_ONLY, UTIL_MOD, WEBPACK_INTERNAL_PREFIX, WORKSHEETS_PREFIX, ZONE_JS,
};

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
    if value.ends_with(EXCEL_XFORM_SUFFIX)
        || value.ends_with(EXCEL_INTERNAL_SUFFIX)
        || value.starts_with(DOC_PROPS_PREFIX)
        || value.starts_with(RELATIONSHIPS_PREFIX)
        || value.starts_with(META_INF_PREFIX)
        || value.starts_with(WORKSHEETS_PREFIX)
        || value.starts_with(THEME_DIR_PREFIX)
        || value.starts_with(WEBPACK_INTERNAL_PREFIX)
        || value == ZONE_JS
        || value.starts_with(READABLE_STREAM_PREFIX)
        || value.starts_with(PROCESS_DIR_PREFIX)
        || value.starts_with(STREAM_DIR_PREFIX)
        || value == BUFFER_MOD
        || value == EVENTS_MOD
        || value == UTIL_MOD
        || value == PATH_MOD
        || value.starts_with(['+', '#'])
        || value.starts_with("${")
        || value == EMPTY_HTTP
        || value.contains(NG_CONTENT)
        || value.starts_with(REF_QUERY_PREFIX)
    {
        return true;
    }
    // regex match (ANY of these)
    [
        &*MODULE_IMPORT,
        &*LOCALE_FILE,
        &*LOCALE_CODE,
        &*SHA_PATTERN,
        &*CRYPTO_MODULE,
        &*PDF_STRUCTURE_1,
        &*PDF_STRUCTURE_2,
        &*PDF_OBJECT_REF,
        &*SINGLE_LETTER_PATH_1,
        &*SINGLE_LETTER_PATH_2,
        &*TIMEZONE_PATTERN,
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

#[must_use]
pub fn is_hostname_noise(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    NOISE_HOSTNAMES
        .iter()
        .any(|n| lower == *n || lower.ends_with(&format!(".{n}")))
}
