use super::fallback::reconstruct_without_sourcemap;
use super::tree::reconstruct_tree;
use std::path::{Component, Path};

const MAX_EXTERNAL_SOURCE_MAP_BYTES: u64 = 32 * 1024 * 1024;

/// Reconstruct a JS bundle's original source tree under `output_dir/reversed/`.
///
/// Uses the bundle's trailing `sourceMappingURL` when present. If the bundle has
/// no source map footer, or its external map file does not exist, falls back to
/// synthetic boundaries: Turbopack module factories, esbuild / Bun lazy module
/// wrappers, else named functions and classes.
///
/// # Errors
/// Returns an error when a referenced map cannot be read/decoded, or when the
/// source tree cannot be written.
pub fn reconstruct_bundle_tree(
    bundle_source: &str,
    bundle_path: Option<&std::path::Path>,
    output_dir: &std::path::Path,
) -> Result<Vec<String>, String> {
    let Some(url) = source_mapping_url(bundle_source) else {
        return reconstruct_without_sourcemap(bundle_source, output_dir);
    };
    if url.starts_with("data:") {
        return reconstruct_tree(&inline_sourcemap_json(url)?, output_dir);
    }
    external_sourcemap_json(url, bundle_path)?.map_or_else(
        || {
            eprintln!("[split-sources] source map {url} not found next to the bundle; using synthetic boundaries");
            reconstruct_without_sourcemap(bundle_source, output_dir)
        },
        |json| reconstruct_tree(&json, output_dir),
    )
}

fn source_mapping_url(bundle_source: &str) -> Option<&str> {
    let marker = "sourceMappingURL=";
    bundle_source.lines().rev().find_map(|line| {
        let (_, raw) = line.split_once(marker)?;
        let value = raw.trim();
        let value = value.strip_suffix("*/").unwrap_or(value).trim();
        value
            .split_whitespace()
            .next()
            .map(|value| value.trim_matches(|c| c == '"' || c == '\''))
    })
}

fn inline_sourcemap_json(url: &str) -> Result<String, String> {
    let (metadata, payload) = url
        .split_once(',')
        .ok_or_else(|| "inline source map data URL has no payload".to_string())?;
    if !metadata.contains(";base64") {
        return Err("inline source map data URL is not base64 encoded".to_string());
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload)
        .map_err(|e| format!("decode inline source map: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("inline source map is not utf8: {e}"))
}

/// The external map's JSON, or `None` when no file exists at its path.
fn external_sourcemap_json(url: &str, bundle_path: Option<&std::path::Path>) -> Result<Option<String>, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Err("remote source maps are not fetched by --split-sources".to_string());
    }
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let map_path = Path::new(path_part);
    if map_path.is_absolute() {
        return Err("absolute source map paths are not read by --split-sources".to_string());
    }
    if map_path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("source map paths may not traverse outside the bundle directory".to_string());
    }
    let bundle_path = bundle_path.ok_or_else(|| "external source map needs a bundle file path".to_string())?;
    let parent = bundle_path.parent().unwrap_or_else(|| Path::new("."));
    let resolved = parent.join(map_path);
    let metadata = match std::fs::metadata(&resolved) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("stat source map {}: {e}", resolved.display())),
    };
    if !metadata.file_type().is_file() {
        return Err(format!("source map {} is not a regular file", resolved.display()));
    }
    if metadata.len() > MAX_EXTERNAL_SOURCE_MAP_BYTES {
        return Err(format!(
            "source map {} exceeds {MAX_EXTERNAL_SOURCE_MAP_BYTES} bytes",
            resolved.display()
        ));
    }
    std::fs::read_to_string(&resolved)
        .map(Some)
        .map_err(|e| format!("read source map {}: {e}", resolved.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstruct_bundle_tree_reads_inline_source_map() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-inline-smtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let map = serde_json::json!({
            "version": 3,
            "sources": ["webpack://app/./src/main.ts"],
            "sourcesContent": ["export const main = 1;"],
            "names": [],
            "mappings": ""
        })
        .to_string();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, map);
        let bundle = format!("console.log(1);\n//# sourceMappingURL=data:application/json;base64,{encoded}");

        let written = reconstruct_bundle_tree(&bundle, None, &dir).unwrap();

        assert_eq!(written, vec!["app/src/main.ts"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("reversed/app/src/main.ts")).unwrap(),
            "export const main = 1;"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_bundle_tree_reads_external_source_map_next_to_bundle() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-external-smtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bundle_path = dir.join("dist/app.js");
        std::fs::create_dir_all(bundle_path.parent().unwrap()).unwrap();
        let map = serde_json::json!({
            "version": 3,
            "sources": ["webpack://app/./src/views/login.tsx"],
            "sourcesContent": ["export function Login() { return null; }"],
            "names": [],
            "mappings": ""
        });
        std::fs::write(dir.join("dist/app.js.map"), map.to_string()).unwrap();
        let bundle = "console.log(1);\n//# sourceMappingURL=app.js.map";

        let written = reconstruct_bundle_tree(bundle, Some(&bundle_path), &dir).unwrap();

        assert_eq!(written, vec!["app/src/views/login.tsx"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("reversed/app/src/views/login.tsx")).unwrap(),
            "export function Login() { return null; }"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_bundle_tree_rejects_absolute_external_source_map_paths() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-absolute-smtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = "console.log(1);\n//# sourceMappingURL=/dev/zero";

        let err = reconstruct_bundle_tree(bundle, None, &dir).unwrap_err();

        assert!(
            err.contains("absolute source map paths are not read"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_bundle_tree_falls_back_when_the_external_map_is_missing() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-missing-smtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle_path = dir.join("app.js");
        let bundle = "function alpha(){return 1;}\n//# sourceMappingURL=app.js.map";

        let written = reconstruct_bundle_tree(bundle, Some(&bundle_path), &dir).unwrap();

        assert_eq!(written, vec!["synthetic/functions/alpha.js".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn split_paths(name: &str, bundle: &str) -> Vec<String> {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-{name}-nosmtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = reconstruct_bundle_tree(bundle, None, &dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        written
    }

    const LAZY_MODULES: &str = concat!(
        "var Q=(e,t)=>()=>(t||e((t={exports:{}}).exports,t),t.exports);",
        "var E=(e,t)=>()=>(e&&(t=e(e=0)),t);",
        "var alpha=Q((exports,module)=>{exports.answer=1});",
        "var beta=E(()=>{console.log('beta')});",
        "alpha();beta();"
    );

    #[test]
    fn reconstruct_bundle_tree_labels_esbuild_modules_without_source_map() {
        let bundle = format!(
            "var d=Object.defineProperty;var X=(e,t)=>{{for(var n in t)d(e,n,{{get:t[n],enumerable:!0}})}};{LAZY_MODULES}"
        );
        let written = split_paths("esbuild", &bundle);
        assert!(
            written.contains(&"synthetic/esbuild/commonjs/alpha.js".to_string()),
            "{written:?}"
        );
        assert!(
            written.contains(&"synthetic/esbuild/esm/beta.js".to_string()),
            "{written:?}"
        );
    }

    #[test]
    fn reconstruct_bundle_tree_leaves_unidentified_lazy_modules_unlabelled() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-lazy-nosmtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let written = reconstruct_bundle_tree(LAZY_MODULES, None, &dir).unwrap();

        assert!(written.contains(&"synthetic/modules/commonjs/alpha.js".to_string()));
        assert!(written.contains(&"synthetic/modules/esm/beta.js".to_string()));
        assert!(
            std::fs::read_to_string(dir.join("reversed/synthetic/modules/commonjs/alpha.js"))
                .unwrap()
                .contains("exports.answer=1")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_bundle_tree_splits_claude_style_bun_wrapper_without_source_map() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-claude-nosmtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = concat!(
            "// @bun @bytecode @bun-cjs\n",
            "(function(exports, require, module, __filename, __dirname) {",
            "var Q=(e,t)=>()=>(t||e((t={exports:{}}).exports,t),t.exports);",
            "var E=(e,t)=>()=>(e&&(t=e(e=0)),t);",
            "var pes=Q(($Nm,nVc)=>{nVc.exports=require('/$bunfs/root/image-processor.node')});",
            "var ges=E(()=>{let version='2.1.195'});",
            "pes();ges();",
            "})"
        );

        let written = reconstruct_bundle_tree(bundle, None, &dir).unwrap();

        assert!(written.contains(&"synthetic/bun/commonjs/pes.js".to_string()));
        assert!(written.contains(&"synthetic/bun/esm/ges.js".to_string()));
        assert!(
            std::fs::read_to_string(dir.join("reversed/synthetic/bun/commonjs/pes.js"))
                .unwrap()
                .contains("image-processor.node")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_bundle_tree_splits_named_boundaries_without_source_map() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-boundary-nosmtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = "function alpha(){return 1;} class Beta { method(){ return alpha(); } }";

        let written = reconstruct_bundle_tree(bundle, None, &dir).unwrap();

        assert_eq!(
            written,
            vec![
                "synthetic/functions/alpha.js".to_string(),
                "synthetic/classes/Beta.js".to_string(),
            ]
        );
        assert!(
            std::fs::read_to_string(dir.join("reversed/synthetic/functions/alpha.js"))
                .unwrap()
                .contains("function alpha()")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconstruct_bundle_tree_preserves_regex_literals_with_braces_in_named_boundaries() {
        let dir = std::env::temp_dir().join(format!("jsb-bundle-regex-nosmtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundle = "function alpha(){const r=/}/; return r.test('}');}";

        let written = reconstruct_bundle_tree(bundle, None, &dir).unwrap();

        assert_eq!(written, vec!["synthetic/functions/alpha.js".to_string()]);
        assert_eq!(
            std::fs::read_to_string(dir.join("reversed/synthetic/functions/alpha.js")).unwrap(),
            bundle
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
