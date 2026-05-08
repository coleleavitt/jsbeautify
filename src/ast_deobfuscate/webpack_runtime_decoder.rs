//! Webpack 5 runtime decoder — renames minified runtime helpers to readable names.
//!
//! Angular CLI + webpack 5.94.0 produces a minified runtime where `__webpack_require__`
//! is renamed to a single letter (typically `t`). This module detects the runtime
//! pattern and replaces all references with human-readable names.
//!
//! ## Mapping (webpack 5.94 as used by Angular CLI 18.2):
//! | Minified | Full Name | Purpose |
//! |----------|-----------|---------|
//! | `t`      | `__webpack_require__` | Module require function |
//! | `t.m`    | `__webpack_require__.modules` | Module registry |
//! | `t.O`    | `__webpack_require__.onChunksLoaded` | Chunk load callbacks |
//! | `t.d`    | `__webpack_require__.definePropertyGetters` | Export definition |
//! | `t.r`    | `__webpack_require__.markAsModule` | Mark ESM |
//! | `t.o`    | `__webpack_require__.hasOwnProperty` | hasOwnProperty helper |
//! | `t.e`    | `__webpack_require__.ensureChunk` | Async chunk loading |
//! | `t.f`    | `__webpack_require__.ensureChunkHandlers` | Chunk handler registry |
//! | `t.f.j`  | `__webpack_require__.ensureChunkHandlers.jsonp` | JSONP chunk loader |
//! | `t.u`    | `__webpack_require__.getChunkScriptFilename` | Chunk filename resolver |
//! | `t.l`    | `__webpack_require__.loadScript` | Script element loader |
//! | `t.p`    | `__webpack_require__.publicPath` | Public path prefix |
//! | `t.t`    | `__webpack_require__.fakeNamespaceObject` | Compat wrapper |
//! | `t.tt`   | `__webpack_require__.trustedTypes` | Trusted Types policy |
//! | `t.tu`   | `__webpack_require__.trustedScriptURL` | Trusted URL factory |
//! | `t.nc`   | `__webpack_require__.nonce` | CSP nonce |
//! | `t.O.j`  | `__webpack_require__.onChunksLoaded.isReady` | Chunk ready check |
//! | `t.miniCssF` | `__webpack_require__.miniCssFilename` | CSS chunk filename |

use regex::Regex;
use rustc_hash::FxHashMap;

/// Mapping from minified webpack runtime property to readable name.
fn build_webpack_runtime_map() -> FxHashMap<&'static str, &'static str> {
    let mut m = FxHashMap::default();
    // Core require function properties
    m.insert(".m", ".modules");
    m.insert(".O", ".onChunksLoaded");
    m.insert(".d", ".definePropertyGetters");
    m.insert(".r", ".markAsModule");
    m.insert(".o", ".hasOwnProperty");
    m.insert(".e", ".ensureChunk");
    m.insert(".f", ".ensureChunkHandlers");
    m.insert(".u", ".getChunkScriptFilename");
    m.insert(".l", ".loadScript");
    m.insert(".p", ".publicPath");
    m.insert(".t", ".fakeNamespaceObject");
    m.insert(".nc", ".nonce");
    m.insert(".tt", ".trustedTypes");
    m.insert(".tu", ".trustedScriptURL");
    m.insert(".miniCssF", ".miniCssFilename");
    // Sub-properties
    m.insert(".O.j", ".onChunksLoaded.isReady");
    m.insert(".f.j", ".ensureChunkHandlers.jsonp");
    m
}

/// Detect the webpack require variable name from a runtime chunk.
/// Looks for the pattern: `function X(e) { var d = v[e]; ... } X.m = m;`
/// or property assignments like `X.m = m;...X.O = ...`
pub fn detect_require_var(source: &str) -> Option<String> {
    lazy_static::lazy_static! {
        // Pattern: single-char function with .exports somewhere after it
        static ref REQUIRE_FUNC_RE: Regex = Regex::new(
            r"function\s+(\w)\([^)]*\)\s*\{.{0,200}\.exports"
        ).unwrap();

        // Pattern: X.m = ...followed by X.O or X.d nearby
        static ref PROP_ASSIGN_RE: Regex = Regex::new(
            r"(\w)\.m\s*=\s*\w+"
        ).unwrap();
    }

    // Strategy: find a single-char function that has .exports in its body,
    // then verify it has .m assigned to it
    if let Some(func_cap) = REQUIRE_FUNC_RE.captures(source) {
        let var_name = func_cap.get(1)?.as_str();
        // Verify this var has .m property assigned (the module registry)
        let check_m = format!("{var_name}.m");
        if source.contains(&check_m) {
            return Some(var_name.to_string());
        }
    }

    // Fallback: look for X.m = ...; X.O = ... pattern
    if let Some(prop_cap) = PROP_ASSIGN_RE.captures(source) {
        let var_name = prop_cap.get(1)?.as_str();
        let check_o = format!("{var_name}.O");
        let check_d = format!("{var_name}.d");
        if source.contains(&check_o) || source.contains(&check_d) {
            return Some(var_name.to_string());
        }
    }

    None
}

/// Decode the webpack runtime: replace minified property accesses with readable names.
/// Returns (decoded_source, replacement_count).
#[must_use]
pub fn decode_webpack_runtime(source: &str) -> (String, usize) {
    let Some(require_var) = detect_require_var(source) else {
        return (source.to_string(), 0);
    };

    let map = build_webpack_runtime_map();
    let mut result = source.to_string();
    let mut count = 0usize;

    // Replace longer patterns first to avoid partial matches
    let mut patterns: Vec<(&str, &str)> = map.iter().map(|(k, v)| (*k, *v)).collect();
    patterns.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    for (minified, readable) in &patterns {
        let from = format!("{require_var}{minified}");
        let to = format!("__webpack_require__{readable}");

        if result.contains(&from) {
            let old_len = result.len();
            result = result.replace(&from, &to);
            if result.len() != old_len {
                count += 1;
            }
        }
    }

    // Also rename the require function itself in common patterns
    // `t(5098)` → `__webpack_require__(5098)`
    let call_pattern = format!(r"\b{}\((\d+)\)", regex::escape(&require_var));
    if let Ok(re) = Regex::new(&call_pattern) {
        let new_result = re.replace_all(&result, "__webpack_require__($1)").to_string();
        if new_result != result {
            count += 1;
            result = new_result;
        }
    }

    // Rename JSONP callback: `self.lmsJsonp` → `self.__webpack_chunk_callback__`
    if result.contains("lmsJsonp") {
        result = result.replace("lmsJsonp", "__webpack_chunk_callback__");
        count += 1;
    }

    (result, count)
}

/// Annotate the webpack runtime with comments explaining each section.
#[must_use]
pub fn annotate_webpack_runtime(source: &str) -> (String, usize) {
    let mut result = String::with_capacity(source.len() + 2048);
    let mut annotations = 0usize;

    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.contains("__webpack_require__.modules") && trimmed.contains("=") {
            result.push_str("// webpack: module registry (all loaded modules stored here)\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_require__.onChunksLoaded") && !trimmed.contains(".isReady") {
            result.push_str("// webpack: deferred module execution (waits for chunks to load)\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_require__.ensureChunk") && !trimmed.contains("Handlers") {
            result.push_str("// webpack: async chunk loading (returns Promise)\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_require__.getChunkScriptFilename") {
            result.push_str("// webpack: chunk ID → filename mapping (content-hashed)\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_require__.loadScript") {
            result.push_str("// webpack: dynamic <script> tag injection for chunk loading\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_require__.markAsModule") {
            result.push_str("// webpack: mark exports as ES Module (sets __esModule = true)\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_require__.trustedTypes") {
            result.push_str("// webpack: Trusted Types policy for CSP compliance\n");
            annotations += 1;
        } else if trimmed.contains("__webpack_chunk_callback__") && trimmed.contains("self.") {
            result.push_str("// webpack: JSONP chunk callback (receives pushed chunk data)\n");
            annotations += 1;
        }

        result.push_str(line);
        result.push('\n');
    }

    (result, annotations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_require_var() {
        let source = r#"function t(e){var d=v[e];if(void 0!==d)return d.exports;var a=v[e]={exports:{}};return m[e].call(a.exports,a,a.exports,t),a.exports}t.m=m;"#;
        assert_eq!(detect_require_var(source), Some("t".to_string()));
    }

    #[test]
    fn test_decode_runtime() {
        let source = "t.m = m;\nt.O = function(){};\nt.d = (e, d) => {};\nt.e = (e) => Promise.all(Object.keys(t.f).reduce((d,a)=>(t.f[a](e,d),d),[]));\nvar x = t(5098);";
        let (decoded, count) = decode_webpack_runtime(source);
        assert!(decoded.contains("__webpack_require__.modules"));
        assert!(decoded.contains("__webpack_require__.onChunksLoaded"));
        assert!(decoded.contains("__webpack_require__.definePropertyGetters"));
        assert!(decoded.contains("__webpack_require__.ensureChunk"));
        assert!(decoded.contains("__webpack_require__(5098)"));
        assert!(count > 0);
    }

    #[test]
    fn test_jsonp_rename() {
        let source = "var a = self.lmsJsonp = self.lmsJsonp || [];";
        let (decoded, _) = decode_webpack_runtime(&format!(
            "function t(e){{var d=v[e];return d.exports}}t.m=m;\n{}",
            source
        ));
        assert!(decoded.contains("__webpack_chunk_callback__"));
    }
}
