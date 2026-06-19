use crate::token::{Token, TokenType};
use crate::{BeautifyError, Result};

pub fn find_module_map_start(tokens: &[Token], index: usize) -> Result<Option<usize>> {
    if !is_jsonp_callee(tokens, index) {
        return Ok(None);
    }
    let call_start = index
        .checked_add(1)
        .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()))?;
    if tokens.get(call_start).map(|t| t.token_type) != Some(TokenType::StartExpr) {
        return Ok(None);
    }
    let Some(map_start) = second_argument_start(tokens, call_start)? else {
        return Ok(None);
    };
    if tokens.get(map_start).map(|t| t.token_type) != Some(TokenType::StartBlock) {
        return Ok(None);
    }
    if is_numeric_module_map(tokens, map_start)? {
        Ok(Some(map_start))
    } else {
        Ok(None)
    }
}

pub fn is_bootstrap_module_array(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).map(|t| t.token_type) == Some(TokenType::StartArray)
        && tokens
            .get(index.saturating_add(1))
            .is_some_and(|t| t.token_type == TokenType::Reserved && t.text == "function")
}

pub fn require_parameter<'a>(tokens: &'a [Token], start: usize, end: usize) -> Option<&'a str> {
    if tokens.get(start)?.token_type != TokenType::Reserved || tokens.get(start)?.text != "function" {
        return None;
    }
    let params_start = start.checked_add(1)?;
    if tokens.get(params_start)?.token_type != TokenType::StartExpr {
        return None;
    }
    let mut params = Vec::with_capacity(3);
    let mut index = params_start.checked_add(1)?;
    while index <= end && index < tokens.len() {
        match tokens[index].token_type {
            TokenType::EndExpr => break,
            TokenType::Word => params.push(tokens[index].text.as_str()),
            _ => {}
        }
        index = index.checked_add(1)?;
    }
    params.get(2).copied()
}

fn is_jsonp_callee(tokens: &[Token], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| {
        token.token_type == TokenType::Word && (token.text.contains("webpackJsonp") || token.text.contains("Jsonp"))
    })
}

fn second_argument_start(tokens: &[Token], call_start: usize) -> Result<Option<usize>> {
    let mut depth = 0usize;
    let mut index = call_start
        .checked_add(1)
        .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()))?;
    while index < tokens.len() {
        match tokens[index].token_type {
            TokenType::Comma if depth == 0 => {
                return index
                    .checked_add(1)
                    .map(Some)
                    .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()));
            }
            TokenType::EndExpr if depth == 0 => return Ok(None),
            token_type if token_type.is_start_delimiter() => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| BeautifyError::BeautificationFailed("depth overflow".to_string()))?;
            }
            token_type if token_type.is_end_delimiter() && depth > 0 => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| BeautifyError::BeautificationFailed("depth underflow".to_string()))?;
            }
            _ => {}
        }
        index = index
            .checked_add(1)
            .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()))?;
    }
    Ok(None)
}

fn is_numeric_module_map(tokens: &[Token], map_start: usize) -> Result<bool> {
    let first_key = map_start
        .checked_add(1)
        .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()))?;
    let colon = first_key
        .checked_add(1)
        .ok_or_else(|| BeautifyError::BeautificationFailed("index overflow".to_string()))?;
    Ok(tokens.get(first_key).map(|t| t.token_type) == Some(TokenType::Number)
        && tokens.get(colon).map(|t| t.token_type) == Some(TokenType::Colon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::Tokenizer;

    fn tokens(code: &str) -> Vec<Token> {
        Tokenizer::new(code).tokenize().expect("tokenize sample")
    }

    #[test]
    fn finds_legacy_jsonp_module_map_normal() {
        let tokens = tokens("webpackJsonp([0],{1:function(t,e,n){n(2);}},[1]);");
        let map_start = find_module_map_start(&tokens, 0).expect("find map");

        assert!(map_start.is_some());
        assert_eq!(tokens[map_start.unwrap()].token_type, TokenType::StartBlock);
    }

    #[test]
    fn reads_third_module_wrapper_parameter_normal() {
        let tokens = tokens("function(t,e,n){n(2);}");

        assert_eq!(require_parameter(&tokens, 0, tokens.len() - 1), Some("n"));
    }

    #[test]
    fn detects_bootstrap_module_array_normal() {
        let tokens = tokens("!function(e){}([function(e,t,n){n(1);},function(e,t){}]);");

        let position = tokens
            .iter()
            .position(|token| token.token_type == TokenType::StartArray)
            .expect("module array");
        assert!(is_bootstrap_module_array(&tokens, position));
    }
}
