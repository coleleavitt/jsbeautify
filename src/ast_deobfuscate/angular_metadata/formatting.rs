use super::extractor::AngularMetadataExtractor;
use super::types::{AngularComponentInfo, AngularDefKind};
use std::cmp::Reverse;

impl AngularMetadataExtractor {
    /// After extraction, annotate the source code with component boundary comments.
    /// Returns the annotated source and the count of annotations added.
    #[must_use]
    pub fn annotate_source(&self, source: &str) -> (String, usize) {
        if self.definitions.is_empty() {
            return (source.to_string(), 0);
        }

        let mut result = String::with_capacity(source.len() + self.definitions.len() * 80);
        let mut annotations = 0usize;

        for line in source.lines() {
            let trimmed = line.trim();

            // Detect component definition lines
            if let Some(info) = self.match_definition_line(trimmed) {
                let kind_label = match info.kind {
                    AngularDefKind::Component => "Component",
                    AngularDefKind::Directive => "Directive",
                    AngularDefKind::Pipe => "Pipe",
                    AngularDefKind::Injectable => "Injectable",
                    AngularDefKind::NgModule => "NgModule",
                };

                result.push_str("// ─── Angular ");
                result.push_str(kind_label);
                result.push_str(": ");
                result.push_str(&info.class_name);
                if !info.selector.is_empty() {
                    result.push_str(" (selector: \"");
                    result.push_str(&info.selector);
                    result.push_str("\")");
                }
                if !info.inputs.is_empty() {
                    result.push_str(" inputs: [");
                    result.push_str(&info.inputs.join(", "));
                    result.push(']');
                }
                if !info.outputs.is_empty() {
                    result.push_str(" outputs: [");
                    result.push_str(&info.outputs.join(", "));
                    result.push(']');
                }
                result.push_str(" ───\n");
                annotations += 1;
            }

            result.push_str(line);
            result.push('\n');
        }

        (result, annotations)
    }

    /// Apply rename map to source code — replaces minified class names with generated names.
    /// Returns (`new_source`, `rename_count`).
    #[must_use]
    pub fn apply_renames(&self, source: &str) -> (String, usize) {
        if self.rename_map.is_empty() {
            return (source.to_string(), 0);
        }

        let mut result = source.to_string();
        let mut count = 0usize;

        // Sort by length descending to avoid partial replacements
        let mut renames: Vec<(&String, &String)> = self.rename_map.iter().collect();
        renames.sort_by_key(|(minified, _)| Reverse(minified.len()));

        for (minified, readable) in renames {
            // Only rename if it appears as a word boundary (not inside another identifier)
            let pattern = format!(r"\b{}\b", regex::escape(minified));
            if let Ok(re) = regex::Regex::new(&pattern) {
                let new_result = re.replace_all(&result, readable.as_str()).to_string();
                if new_result != result {
                    count += 1;
                    result = new_result;
                }
            }
        }

        (result, count)
    }

    pub(super) fn build_rename_map(&mut self) {
        for info in &self.definitions {
            if let Some(ref export_name) = info.minified_export_name {
                // Only rename short minified names (1-3 chars are definitely minified)
                if export_name.len() <= 3 && !info.class_name.is_empty() {
                    self.rename_map.insert(export_name.clone(), info.class_name.clone());
                }
            }
            if let Some(ref class_name) = info.minified_class_name
                && class_name.len() <= 3
                && !info.class_name.is_empty()
            {
                self.rename_map.insert(class_name.clone(), info.class_name.clone());
            }
        }
    }

    fn match_definition_line<'a>(&'a self, line: &str) -> Option<&'a AngularComponentInfo> {
        for info in &self.definitions {
            // Match on the selector appearing in a defineComponent/defineDirective call
            if (line.contains("defineComponent") || line.contains("defineDirective"))
                && let Some(sel) = Some(&info.selector)
                && !sel.is_empty()
                && line.contains(sel.as_str())
            {
                return Some(info);
            }
            if line.contains("definePipe") && line.contains(&info.selector) {
                return Some(info);
            }
        }
        None
    }
}
