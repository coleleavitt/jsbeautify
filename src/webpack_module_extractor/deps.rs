use super::{ModuleExtractor, legacy_jsonp};
use crate::token::{Token, TokenType};
use crate::{BeautifyError, Result};

impl ModuleExtractor {
    /// # Errors
    /// Returns an error if token scanning overflows while collecting dependencies.
    pub fn extract_dependencies(&mut self, tokens: &[Token]) -> Result<()> {
        trace_webpack!("=== EXTRACTING DEPENDENCIES ===");

        for (id, module) in &self.modules.clone() {
            let deps = find_dependencies_in_range(tokens, module.start_pos, module.end_pos)?;

            if let Some(module) = self.modules.get_mut(id) {
                module.dependencies.clone_from(&deps);
                trace_webpack!("module {} has {} dependencies", id, deps.len());
            }
        }

        Ok(())
    }
}

fn find_dependencies_in_range(tokens: &[Token], start: usize, end: usize) -> Result<Vec<usize>> {
    let mut deps = Vec::new();
    let mut index = start;
    let require_name = legacy_jsonp::require_parameter(tokens, start, end).unwrap_or("t");

    while index <= end && index < tokens.len() {
        if tokens[index].token_type == TokenType::Word && tokens[index].text == require_name {
            let paren = index
                .checked_add(1)
                .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

            if paren < tokens.len() && tokens[paren].token_type == TokenType::StartExpr {
                let num_pos = paren
                    .checked_add(1)
                    .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

                if num_pos < tokens.len()
                    && tokens[num_pos].token_type == TokenType::Number
                    && let Ok(dep_id) = tokens[num_pos].text.parse::<usize>()
                    && !deps.contains(&dep_id)
                {
                    deps.push(dep_id);
                }
            }
        }

        index = index
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
    }

    Ok(deps)
}
