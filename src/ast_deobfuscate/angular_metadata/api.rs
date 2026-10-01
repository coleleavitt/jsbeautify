use super::extractor::AngularMetadataExtractor;
use super::regex_fallback::extract_via_regex;
use super::types::{AngularDefKind, AngularExtractionResult};
use super::version::detect_angular_version;
use crate::recon_budget::ReconBudget;

/// High-level: run full Angular extraction on source text.
/// Returns (`annotated_source`, stats).
#[must_use]
pub fn extract_angular_metadata(source: &str) -> AngularExtractionResult {
    extract_angular_metadata_with_limit(source, usize::MAX)
}

/// Extract while retaining at most `max_findings` definitions.
#[must_use]
pub fn extract_angular_metadata_with_limit(source: &str, max_findings: usize) -> AngularExtractionResult {
    let mut budget = ReconBudget::new(usize::MAX);
    extract_angular_metadata_with_budget(source, max_findings, &mut budget)
}

#[must_use]
pub fn extract_angular_metadata_with_budget(
    source: &str,
    max_findings: usize,
    budget: &mut ReconBudget,
) -> AngularExtractionResult {
    let version = detect_angular_version(source);

    let allocator = oxc_allocator::Allocator::new();
    let source_type = oxc_span::SourceType::default();
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type).parse();

    let mut extractor = AngularMetadataExtractor::with_limit(max_findings);

    if parsed.diagnostics.is_empty() && !parsed.program.body.is_empty() {
        // AST-based extraction (preferred — gives inputs/outputs/full metadata)
        extractor.extract_from_statements(&parsed.program.body, budget);
    }

    // If AST extraction found nothing (parse errors, empty body, etc.),
    // fall back to regex-based extraction which works on any text.
    if extractor.definitions.is_empty() && source.contains("defineComponent") {
        eprintln!("[ANGULAR] AST parse failed or found nothing, using regex fallback");
        extract_via_regex(source, &mut extractor, budget);
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

    let annotation_payload = extractor.definitions.iter().fold(source.len(), |total, definition| {
        let strings = definition.class_name.len()
            + definition.selector.len()
            + definition.inputs.iter().map(String::len).sum::<usize>()
            + definition.outputs.iter().map(String::len).sum::<usize>();
        total.saturating_add(strings)
    });
    let (annotated, annotation_count) = if budget.try_reserve(annotation_payload) {
        extractor.annotate_source(source)
    } else {
        (String::new(), 0)
    };

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
