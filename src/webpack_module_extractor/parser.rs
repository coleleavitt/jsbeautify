use super::{ModuleExtractor, WebpackModule, legacy_jsonp};
use crate::token::{Token, TokenType};
use crate::{BeautifyError, Result};

impl ModuleExtractor {
    pub(super) fn find_module_map(&mut self, tokens: &[Token]) -> Result<()> {
        let mut index = 0usize;

        while index < tokens.len() {
            if let Some(map_start) = self.try_find_module_map_at(tokens, index)? {
                trace_webpack!("found module map at position {}", map_start);
                let before = self.modules.len();
                self.parse_module_map(tokens, map_start)?;
                if self.modules.len() > before {
                    return Ok(());
                }
            }

            index = index
                .checked_add(1)
                .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
        }

        trace_webpack!("no module map found");
        Ok(())
    }

    fn try_find_module_map_at(&self, tokens: &[Token], index: usize) -> Result<Option<usize>> {
        if let Some(map_start) = legacy_jsonp::find_module_map_start(tokens, index)? {
            return Ok(Some(map_start));
        }
        if legacy_jsonp::is_bootstrap_module_array(tokens, index) {
            return Ok(Some(index));
        }

        if index
            .checked_add(10)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?
            >= tokens.len()
        {
            return Ok(None);
        }

        if tokens[index].token_type != TokenType::Equals {
            return Ok(None);
        }

        let next = index
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

        if tokens[next].token_type != TokenType::StartBlock {
            return Ok(None);
        }

        let after_brace = next
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

        if after_brace >= tokens.len() {
            return Ok(None);
        }

        if tokens[after_brace].token_type == TokenType::Number {
            let colon_pos = after_brace
                .checked_add(1)
                .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

            if colon_pos < tokens.len() && tokens[colon_pos].token_type == TokenType::Colon {
                return Ok(Some(next));
            }
        }

        Ok(None)
    }

    fn parse_module_map(&mut self, tokens: &[Token], start: usize) -> Result<()> {
        match tokens[start].token_type {
            TokenType::StartBlock => self.parse_object_module_map(tokens, start),
            TokenType::StartArray => self.parse_array_module_map(tokens, start),
            _ => Ok(()),
        }
    }

    fn parse_object_module_map(&mut self, tokens: &[Token], start: usize) -> Result<()> {
        debug_assert_eq!(tokens[start].token_type, TokenType::StartBlock);

        let mut index = start
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

        while index < tokens.len() {
            if tokens[index].token_type == TokenType::EndBlock {
                break;
            }

            if tokens[index].token_type == TokenType::Number
                && let Some(module) = self.try_parse_module(tokens, index)?
            {
                let next = module
                    .end_pos
                    .checked_add(1)
                    .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
                self.modules.insert(module.id, module);
                index = next;
                continue;
            }

            index = index
                .checked_add(1)
                .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
        }

        Ok(())
    }

    fn parse_array_module_map(&mut self, tokens: &[Token], start: usize) -> Result<()> {
        debug_assert_eq!(tokens[start].token_type, TokenType::StartArray);

        let mut module_id = 0usize;
        let mut index = start
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

        while index < tokens.len() {
            match tokens[index].token_type {
                TokenType::EndArray => break,
                TokenType::Comma => {
                    module_id = module_id
                        .checked_add(1)
                        .ok_or_else(|| BeautifyError::BeautificationFailed("module id overflow".to_string()))?;
                    index = index
                        .checked_add(1)
                        .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
                }
                TokenType::Reserved if tokens[index].text == "function" => {
                    if let Some(func_end) = find_function_end(tokens, index)? {
                        trace_webpack!("parsed module {}: positions {}..{}", module_id, index, func_end);
                        self.modules.insert(
                            module_id,
                            WebpackModule {
                                id: module_id,
                                start_pos: index,
                                end_pos: func_end,
                                dependencies: Vec::new(),
                            },
                        );
                        index = func_end
                            .checked_add(1)
                            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
                    } else {
                        index = index
                            .checked_add(1)
                            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
                    }
                }
                _ => {
                    index = index
                        .checked_add(1)
                        .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;
                }
            }
        }

        Ok(())
    }

    fn try_parse_module(&self, tokens: &[Token], index: usize) -> Result<Option<WebpackModule>> {
        if tokens[index].token_type != TokenType::Number {
            return Ok(None);
        }

        let Some(module_id) = tokens[index].text.parse::<usize>().ok() else {
            return Ok(None);
        };

        let colon_pos = index
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

        if colon_pos >= tokens.len() || tokens[colon_pos].token_type != TokenType::Colon {
            return Ok(None);
        }

        let func_pos = colon_pos
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("overflow".to_string()))?;

        if func_pos >= tokens.len() {
            return Ok(None);
        }

        if tokens[func_pos].token_type == TokenType::Reserved && tokens[func_pos].text == "function" {
            let Some(func_end) = find_function_end(tokens, func_pos)? else {
                return Ok(None);
            };

            trace_webpack!("parsed module {}: positions {}..{}", module_id, func_pos, func_end);

            return Ok(Some(WebpackModule {
                id: module_id,
                start_pos: func_pos,
                end_pos: func_end,
                dependencies: Vec::new(),
            }));
        }

        Ok(None)
    }
}

pub(super) fn find_function_end(tokens: &[Token], start: usize) -> Result<Option<usize>> {
    debug_assert_eq!(tokens[start].token_type, TokenType::Reserved);
    debug_assert_eq!(tokens[start].text.as_str(), "function");

    let mut index = start;
    let mut depth = 0usize;
    let mut found_first_brace = false;

    while index < tokens.len() {
        if tokens[index].token_type == TokenType::StartBlock {
            depth = depth
                .checked_add(1)
                .ok_or_else(|| BeautifyError::BeautificationFailed("depth overflow".to_string()))?;
            found_first_brace = true;
        } else if tokens[index].token_type == TokenType::EndBlock && depth > 0 {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| BeautifyError::BeautificationFailed("depth underflow".to_string()))?;

            if found_first_brace && depth == 0 {
                return Ok(Some(index));
            }
        }

        index = index
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()))?;
    }

    Ok(None)
}
