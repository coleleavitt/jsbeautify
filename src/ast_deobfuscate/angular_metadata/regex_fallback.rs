use super::extractor::AngularMetadataExtractor;
use super::naming::generate_class_name;
use super::types::{AngularComponentInfo, AngularDefKind};

/// Regex-based fallback extraction for when AST parsing fails
/// (e.g. beautified output has duplicate identifiers across inlined modules).
pub(super) fn extract_via_regex(source: &str, extractor: &mut AngularMetadataExtractor) {
    use regex::Regex;
    use std::sync::LazyLock;

    // Matches: selectors: [["component-selector"]]
    static SELECTOR_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:defineComponent|defineDirective)\(\{[^}]*?selectors:\s*\[\["([^"]+)"\]\]"#).unwrap()
    });

    // Matches: definePipe({name: "pipeName", ...})
    static PIPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"definePipe\(\{[^}]*?name:\s*"([^"]+)""#).unwrap());

    // Extract components/directives via selectors
    for caps in SELECTOR_RE.captures_iter(source) {
        let selector = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let full_match = caps.get(0).map_or("", |m| m.as_str());
        let kind = if full_match.contains("defineDirective") {
            AngularDefKind::Directive
        } else {
            AngularDefKind::Component
        };
        let class_name = generate_class_name(&selector, kind);

        extractor.definitions.push(AngularComponentInfo {
            selector,
            class_name,
            kind,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    // Extract pipes
    for caps in PIPE_RE.captures_iter(source) {
        let pipe_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let class_name = generate_class_name(&pipe_name, AngularDefKind::Pipe);

        extractor.definitions.push(AngularComponentInfo {
            selector: pipe_name,
            class_name,
            kind: AngularDefKind::Pipe,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    // Count injectables
    for _ in source.match_indices("defineInjectable({") {
        extractor.definitions.push(AngularComponentInfo {
            selector: String::new(),
            class_name: String::new(),
            kind: AngularDefKind::Injectable,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    // Count NgModules
    for _ in source.match_indices("defineNgModule({") {
        extractor.definitions.push(AngularComponentInfo {
            selector: String::new(),
            class_name: String::new(),
            kind: AngularDefKind::NgModule,
            inputs: Vec::new(),
            outputs: Vec::new(),
            minified_export_name: None,
            minified_class_name: None,
        });
    }

    extractor.build_rename_map();
}
