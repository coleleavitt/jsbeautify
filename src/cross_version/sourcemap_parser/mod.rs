//! Sourcemap parsing and name extraction via AST-based identifier extraction.
//!
//! Uses oxc parser to extract all identifiers with precise positions,
//! then maps them to original names via sourcemap lookup.

mod identifiers;
mod mappings;
#[cfg(test)]
mod tests;
mod vlq;

use crate::{BeautifyError, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct NameMapping {
    pub minified_name: String,
    pub original_name: String,
    pub source_file: String,
    pub original_line: u32,
    pub original_column: u32,
    pub minified_line: u32,
    pub minified_column: u32,
}

#[derive(Debug, Deserialize)]
struct RawSourcemap {
    sources: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    names: Vec<String>,
    mappings: String,
    #[serde(rename = "sourcesContent", default)]
    sources_content: Vec<Option<String>>,
}

pub struct SourcemapParser {
    identifier_pattern: regex::Regex,
}

impl SourcemapParser {
    #[must_use]
    /// # Panics
    /// Panics only if the built-in JavaScript identifier regex is invalid.
    pub fn new() -> Self {
        Self {
            identifier_pattern: regex::Regex::new(r"^[a-zA-Z_$][a-zA-Z0-9_$]*").expect("valid identifier regex"),
        }
    }

    /// # Errors
    /// Returns an error when the sourcemap JSON or VLQ mappings are invalid.
    pub fn extract_names(&self, sourcemap_json: &str, bundle_source: &str) -> Result<Vec<NameMapping>> {
        let raw: RawSourcemap = serde_json::from_str(sourcemap_json)
            .map_err(|e| BeautifyError::InvalidInput(format!("Invalid sourcemap JSON: {e}")))?;

        let mapping_index = self.build_mapping_index(&raw.mappings, raw.sources.len())?;
        let bundle_identifiers =
            crate::stack::run_on_sized_stack(bundle_source.len(), || self.extract_identifiers_ast(bundle_source));

        eprintln!(
            "[SOURCEMAP] {} identifiers, {} mapping lines",
            bundle_identifiers.len(),
            mapping_index.len()
        );

        let mut mappings = Vec::new();

        for (line, col, minified_name) in &bundle_identifiers {
            let Some(entry) = self.lookup_original_position(&mapping_index, *line, *col) else {
                continue;
            };

            let Some(source_content) = raw.sources_content.get(entry.source_idx).and_then(Option::as_ref) else {
                continue;
            };

            let Some(original_name) = self.get_identifier_at(source_content, entry.orig_line, entry.orig_col) else {
                continue;
            };

            mappings.push(NameMapping {
                minified_name: minified_name.clone(),
                original_name,
                source_file: raw.sources.get(entry.source_idx).cloned().unwrap_or_default(),
                original_line: entry.orig_line,
                original_column: entry.orig_col,
                minified_line: *line,
                minified_column: *col,
            });
        }

        eprintln!("[SOURCEMAP] {} raw name mappings extracted", mappings.len());
        Ok(mappings)
    }
}

impl Default for SourcemapParser {
    fn default() -> Self {
        Self::new()
    }
}
