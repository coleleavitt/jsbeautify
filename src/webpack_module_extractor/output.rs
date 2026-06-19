use super::ModuleExtractor;
use crate::token::Token;
use crate::{BeautifyError, Result};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

impl ModuleExtractor {
    /// # Errors
    /// Returns an error if the output directory or module files cannot be written.
    pub fn write_modules(&self, tokens: &[Token], output_dir: &Path) -> Result<()> {
        trace_webpack!("=== WRITING MODULES ===");
        trace_webpack!("output directory: {}", output_dir.display());

        fs::create_dir_all(output_dir)
            .map_err(|e| BeautifyError::BeautificationFailed(format!("failed to create output directory: {e}")))?;

        for (id, module) in &self.modules {
            let filename = format!("module_{id}.js");
            let output_path = output_dir.join(&filename);
            let module_code = tokens_to_string(&tokens[module.start_pos..=module.end_pos]);

            fs::write(&output_path, module_code)
                .map_err(|e| BeautifyError::BeautificationFailed(format!("failed to write module {id}: {e}")))?;

            trace_webpack!("wrote module {} to {}", id, filename);
        }

        trace_webpack!("wrote {} modules", self.modules.len());
        Ok(())
    }

    /// # Errors
    /// Returns an error if the dependency graph file cannot be written.
    pub fn generate_dependency_graph(&self, output_path: &Path) -> Result<()> {
        trace_webpack!("=== GENERATING DEPENDENCY GRAPH ===");

        let mut graph = String::from("digraph webpack_modules {\n");
        graph.push_str("  rankdir=LR;\n");
        graph.push_str("  node [shape=box];\n\n");

        for id in self.modules.keys() {
            let _ = writeln!(graph, "  module_{id} [label=\"Module {id}\"];");
        }

        graph.push('\n');

        for (id, module) in &self.modules {
            for dep in &module.dependencies {
                let _ = writeln!(graph, "  module_{id} -> module_{dep};");
            }
        }

        graph.push_str("}\n");

        fs::write(output_path, graph)
            .map_err(|e| BeautifyError::BeautificationFailed(format!("failed to write dependency graph: {e}")))?;

        trace_webpack!("wrote dependency graph to {}", output_path.display());
        Ok(())
    }
}

fn tokens_to_string(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
