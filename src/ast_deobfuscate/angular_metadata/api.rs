use super::extractor::AngularMetadataExtractor;
use super::regex_fallback::extract_via_regex;
use super::types::{AngularDefKind, AngularExtractionResult};
use super::version::detect_angular_version;

/// High-level: run full Angular extraction on source text.
/// Returns (`annotated_source`, stats).
#[must_use]
pub fn extract_angular_metadata(source: &str) -> AngularExtractionResult {
    let version = detect_angular_version(source);

    let allocator = oxc_allocator::Allocator::new();
    let source_type = oxc_span::SourceType::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let mut extractor = AngularMetadataExtractor::new();

    if parsed.diagnostics.is_empty() && !parsed.program.body.is_empty() {
        // AST-based extraction (preferred — gives inputs/outputs/full metadata)
        extractor.extract_from_statements(&parsed.program.body);
    }

    // If AST extraction found nothing (parse errors, empty body, etc.),
    // fall back to regex-based extraction which works on any text.
    if extractor.definitions.is_empty() && source.contains("defineComponent") {
        eprintln!("[ANGULAR] AST parse failed or found nothing, using regex fallback");
        extract_via_regex(source, &mut extractor);
    }

    let component_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Component)
        .count();
    let directive_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Directive)
        .count();
    let pipe_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Pipe)
        .count();
    let injectable_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::Injectable)
        .count();
    let module_count = extractor
        .definitions
        .iter()
        .filter(|d| d.kind == AngularDefKind::NgModule)
        .count();

    let (annotated, annotation_count) = extractor.annotate_source(source);

    AngularExtractionResult {
        source: annotated,
        version,
        component_count,
        directive_count,
        pipe_count,
        injectable_count,
        module_count,
        annotation_count,
        definitions: extractor.definitions,
        rename_map: extractor.rename_map,
    }
}
