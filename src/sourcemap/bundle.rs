use super::tree::reconstruct_tree;

/// Reconstruct a JS bundle's original source tree under `output_dir/reversed/`.
///
/// Reads the bundle's trailing `sourceMappingURL`, supports inline base64 data
/// URLs and local external `.map` files next to `bundle_path`, then delegates to
/// [`reconstruct_tree`].
///
/// # Errors
/// Returns an error when the bundle has no source map reference, the referenced
/// map cannot be read/decoded, or the source map cannot be reconstructed.
pub fn reconstruct_bundle_tree(
    bundle_source: &str,
    bundle_path: Option<&std::path::Path>,
    output_dir: &std::path::Path,
) -> Result<Vec<String>, String> {
    let map_json = bundle_sourcemap_json(bundle_source, bundle_path)?;
    reconstruct_tree(&map_json, output_dir)
}

fn bundle_sourcemap_json(bundle_source: &str, bundle_path: Option<&std::path::Path>) -> Result<String, String> {
    let url = source_mapping_url(bundle_source).ok_or_else(|| "bundle has no sourceMappingURL".to_string())?;
    if url.starts_with("data:") {
        return inline_sourcemap_json(url);
    }
    external_sourcemap_json(url, bundle_path)
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

fn external_sourcemap_json(url: &str, bundle_path: Option<&std::path::Path>) -> Result<String, String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Err("remote source maps are not fetched by --split-sources".to_string());
    }
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let map_path = std::path::Path::new(path_part);
    let resolved = if map_path.is_absolute() {
        map_path.to_path_buf()
    } else {
        let bundle_path = bundle_path.ok_or_else(|| "external source map needs a bundle file path".to_string())?;
        let parent = bundle_path.parent().unwrap_or_else(|| std::path::Path::new("."));
        parent.join(map_path)
    };
    std::fs::read_to_string(&resolved).map_err(|e| format!("read source map {}: {e}", resolved.display()))
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
}
