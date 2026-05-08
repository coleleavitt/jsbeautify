//! Angular route extractor — maps webpack chunk IDs to URL route paths.
//!
//! Angular's lazy-loaded routes produce patterns like:
//! ```js
//! { path: "hard-token", loadChildren: () => __webpack_require__.ensureChunk(3832).then(...) }
//! { path: "mfa-enrollment", loadComponent: () => import(/* webpackChunkName: "7794" */ ...) }
//! ```
//!
//! This module extracts the route→chunk mapping to produce a route table like:
//! - `/hard-token` → chunk 3832 (HardTokenModule)
//! - `/mfa-enrollment` → chunk 7794 (MfaEnrollmentModule)
//! - `/update-password` → chunk 6848 (UpdatePasswordModule)

use regex::Regex;

/// A single extracted route entry.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub path: String,
    pub chunk_id: Option<u32>,
    pub module_name: Option<String>,
    pub is_lazy: bool,
}

/// Result of route extraction.
#[derive(Debug)]
pub struct RouteExtractionResult {
    pub routes: Vec<RouteEntry>,
    pub chunk_to_route: Vec<(u32, String)>,
}

/// Extract Angular route definitions from beautified/deobfuscated source.
/// Parses `path:` + `loadChildren`/`loadComponent` patterns to build the route table.
#[must_use]
pub fn extract_routes(source: &str) -> RouteExtractionResult {
    let mut routes = Vec::new();
    let mut chunk_to_route: Vec<(u32, String)> = Vec::new();

    lazy_static::lazy_static! {
        // path: "some-path", loadChildren: () => ...ensureChunk(NNNN)...
        static ref ROUTE_ENSURE_RE: Regex = Regex::new(
            r#"path:\s*"([^"]*)"[^}]*?(?:loadChildren|loadComponent)\s*:\s*\(\)\s*=>[^}]*?(?:ensureChunk|\.e)\((\d+)\)"#
        ).unwrap();

        // path: "some-path", loadChildren: () => import(...)...then(m => m.SomeModule)
        static ref ROUTE_IMPORT_RE: Regex = Regex::new(
            r#"path:\s*"([^"]*)"[^}]*?(?:loadChildren|loadComponent)\s*:\s*\(\)\s*=>.*?\.then\(\w+\s*=>\s*\w+\.(\w+)\)"#
        ).unwrap();

        // Simpler pattern: path: "x" appearing near a chunk number
        static ref PATH_CHUNK_RE: Regex = Regex::new(
            r#"path:\s*"([^"]+)"[^}]{0,200}?(\d{3,5})\)"#
        ).unwrap();

        // Module name in .then(m => m.XXXModule)
        static ref MODULE_NAME_RE: Regex = Regex::new(
            r#"\.then\(\w+\s*=>\s*\w+\.(\w+Module)\)"#
        ).unwrap();
    }

    // First pass: ensureChunk patterns
    for caps in ROUTE_ENSURE_RE.captures_iter(source) {
        let path = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let chunk_id: Option<u32> = caps.get(2).and_then(|m| m.as_str().parse().ok());

        if let Some(id) = chunk_id {
            chunk_to_route.push((id, path.clone()));
        }

        routes.push(RouteEntry {
            path,
            chunk_id,
            module_name: None,
            is_lazy: true,
        });
    }

    // Second pass: import().then(m => m.Module) patterns
    for caps in ROUTE_IMPORT_RE.captures_iter(source) {
        let path = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let module_name = caps.get(2).map(|m| m.as_str().to_string());

        // Check if we already have this route from the first pass
        let existing = routes.iter_mut().find(|r| r.path == path);
        if let Some(entry) = existing {
            entry.module_name = module_name;
        } else {
            routes.push(RouteEntry {
                path,
                chunk_id: None,
                module_name,
                is_lazy: true,
            });
        }
    }

    // Third pass: fallback for simpler path patterns
    if routes.is_empty() {
        for caps in PATH_CHUNK_RE.captures_iter(source) {
            let path = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let chunk_id: Option<u32> = caps.get(2).and_then(|m| m.as_str().parse().ok());

            if let Some(id) = chunk_id {
                chunk_to_route.push((id, path.clone()));
            }

            routes.push(RouteEntry {
                path,
                chunk_id,
                module_name: None,
                is_lazy: true,
            });
        }
    }

    // Deduplicate chunk_to_route
    chunk_to_route.sort_by_key(|(id, _)| *id);
    chunk_to_route.dedup_by_key(|(id, _)| *id);

    RouteExtractionResult { routes, chunk_to_route }
}

/// Generate a human-readable route table as a comment block.
#[must_use]
pub fn format_route_table(result: &RouteExtractionResult) -> String {
    if result.routes.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    output.push_str("// ║           ANGULAR ROUTE → CHUNK MAPPING                 ║\n");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");

    for route in &result.routes {
        output.push_str("// ║ ");
        let path_display = if route.path.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", route.path)
        };

        let chunk_display = route.chunk_id.map_or_else(|| "???".to_string(), |id| format!("{id}"));

        let module_display = route.module_name.as_deref().unwrap_or("");

        output.push_str(&format!(
            "{:<30} → chunk {:<6} {}\n",
            path_display, chunk_display, module_display
        ));
    }

    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_ensure_chunk_routes() {
        let source = r#"
            {path: "hard-token", loadChildren: () => __webpack_require__.ensureChunk(3832).then(m => m.HardTokenModule)},
            {path: "mfa-enrollment", loadChildren: () => __webpack_require__.ensureChunk(7794).then(m => m.MfaEnrollmentModule)},
            {path: "update-password", loadChildren: () => __webpack_require__.ensureChunk(6848).then(m => m.UpdatePasswordModule)},
        "#;

        let result = extract_routes(source);
        assert_eq!(result.routes.len(), 3);
        assert_eq!(result.routes[0].path, "hard-token");
        assert_eq!(result.routes[0].chunk_id, Some(3832));
        assert_eq!(result.routes[1].path, "mfa-enrollment");
        assert_eq!(result.routes[1].chunk_id, Some(7794));

        let table = format_route_table(&result);
        assert!(table.contains("/hard-token"));
        assert!(table.contains("3832"));
    }

    #[test]
    fn test_extract_minified_routes() {
        let source = r#"
            {path:"confirm-identity",loadChildren:()=>t.e(9178).then(t.t(4539,23))},
            {path:"totp",loadChildren:()=>t.e(682).then(t.t(1121,23))},
        "#;

        let result = extract_routes(source);
        assert_eq!(result.routes.len(), 2);
        assert_eq!(result.routes[0].path, "confirm-identity");
        assert_eq!(result.routes[0].chunk_id, Some(9178));
    }
}
