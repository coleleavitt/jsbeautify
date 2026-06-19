use super::parser::TemplateDecompiler;

/// Decompile an Angular component from its beautified source text.
/// Extracts template instructions and reconstructs approximate HTML.
///
/// Input: the full beautified source of a defineComponent call region.
/// Returns: reconstructed pseudo-template and metadata.
#[must_use]
pub fn decompile_template_from_source(source: &str) -> Option<String> {
    let mut decompiler = TemplateDecompiler::new();
    let mut in_create = false;
    let mut in_update = false;

    for line in source.lines() {
        let trimmed = line.trim();

        // Detect create phase: `if (1 & rf)` or `if (1 & dt)`
        if trimmed.contains("1 & ") && trimmed.contains("if") {
            in_create = true;
            in_update = false;
            continue;
        }
        // Detect update phase: `if (2 & rf)` or `if (2 & dt)`
        if trimmed.contains("2 & ") && trimmed.contains("if") {
            in_create = false;
            in_update = true;
            continue;
        }

        if in_create {
            decompiler.process_create_instruction(trimmed);
        } else if in_update {
            decompiler.process_update_instruction(trimmed);
        }
    }

    let html = decompiler.emit_html();
    if html.is_empty() { None } else { Some(html) }
}
