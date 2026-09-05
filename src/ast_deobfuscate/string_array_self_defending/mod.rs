//! Self-defending string array pattern (javascript-obfuscator modern default)
//!
//! Handles the three-piece pattern produced by javascript-obfuscator with
//! `selfDefending: true` + `rotateStringArray: true`:
//!
//! 1. **Self-replacing array wrapper**: `function NAME() { const arr = [...]; NAME = function(){return arr}; return NAME(); }`
//! 2. **Decoder function**: `function NAME(idx, _key) { idx = idx - OFFSET; const arr = WRAPPER(); return arr[idx]; }`
//! 3. **Checksum rotation IIFE**: brute-forces `arr.push(arr.shift())` until a
//!    `parseInt`-based checksum matches a target literal.
//!
//! All three pieces can appear in any order in the source. Detection collects
//! candidates without cross-referencing; `finalize()` resolves relationships.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod array_extraction;
mod checksum;
mod decoder_ref;
mod detection;
mod pass;
mod rotation;
mod types;

pub use pass::SelfDefendingStringArray;

#[cfg(test)]
use crate::ast_deobfuscate::state::OffsetOperation;
#[cfg(test)]
use rotation::{brute_force_rotation, parse_int_js};
#[cfg(test)]
use types::ChecksumTerm;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::{DecoderInliner, DeobfuscateState, StringArrayInliner};
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    fn run_self_defending(code: &str) -> (String, DeobfuscateState) {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(ret.diagnostics.is_empty(), "Parse errors: {:?}", ret.diagnostics);
        let mut program = ret.program;

        let mut pass = SelfDefendingStringArray::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);
        oxc_traverse::traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);

        let mut state = DeobfuscateState::new();
        pass.finalize(&mut state);

        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);

        let mut decoder = DecoderInliner::new();
        oxc_traverse::traverse_mut_with_ctx(&mut decoder, &mut program, &mut ctx);

        let mut inliner = StringArrayInliner::new();
        oxc_traverse::traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);

        let output = Codegen::new().build(&program).code;
        let state = ctx.into_state();
        (output, state)
    }

    #[test]
    fn test_self_defending_fixture_zero_rotation() {
        let code = r#"
'use strict';
const dec = _0x5eba;
(function(arrFn, target) {
    const d = _0x5eba;
    const arr = arrFn();
    while (true) {
        try {
            const c = parseInt(d(0x100)) / 0x1
                    + parseInt(d(0x101)) / 0x2
                    - parseInt(d(0x102)) / 0x3;
            if (c === target) break;
            else arr.push(arr.shift());
        } catch (e) { arr.push(arr.shift()); }
    }
}(_0x3ad3, 180));
function getKey() { return dec(0x104); }
function _0x3ad3() {
    const arr = ["100", "200", "60", "https://abc@sentry.io/1",
                 "phc_PUBLIC_KEY_VALUE_HERE", "app.example.com:443", "extra-payload"];
    _0x3ad3 = function() { return arr; };
    return _0x3ad3();
}
function _0x5eba(idx, _key) {
    idx = idx - 0x100;
    const arr = _0x3ad3();
    return arr[idx];
}
"#;

        let (output, state) = run_self_defending(code);
        eprintln!("Output:\n{output}");

        assert!(
            state.string_arrays.contains_key("_0x3ad3"),
            "Should detect _0x3ad3 as string array wrapper"
        );

        assert!(
            state.decoders.contains_key("_0x5eba"),
            "Should detect _0x5eba as decoder"
        );

        assert!(
            output.contains("\"phc_PUBLIC_KEY_VALUE_HERE\""),
            "getKey() should contain the inlined PostHog key. Output:\n{output}"
        );
    }

    #[test]
    fn test_parse_int_js() {
        assert_eq!(parse_int_js("100"), Some(100.0));
        assert_eq!(parse_int_js("200"), Some(200.0));
        assert_eq!(parse_int_js("60"), Some(60.0));
        assert_eq!(parse_int_js("0xFF"), Some(255.0));
        assert_eq!(parse_int_js("123abc"), Some(123.0));
        assert_eq!(parse_int_js("abc"), None);
        assert_eq!(parse_int_js(""), None);
        assert_eq!(parse_int_js("-42"), Some(-42.0));
    }

    #[test]
    fn test_brute_force_rotation_zero() {
        let strings = vec![
            "100".into(),
            "200".into(),
            "60".into(),
            "sentry".into(),
            "phc_key".into(),
            "host".into(),
            "extra".into(),
        ];
        let terms = vec![
            ChecksumTerm {
                indices: vec![0x100],
                divisors: vec![1.0],
                sign: 1,
            },
            ChecksumTerm {
                indices: vec![0x101],
                divisors: vec![2.0],
                sign: 1,
            },
            ChecksumTerm {
                indices: vec![0x102],
                divisors: vec![3.0],
                sign: -1,
            },
        ];

        let result = brute_force_rotation(&strings, &terms, 180.0, 0x100, OffsetOperation::Subtract);
        assert_eq!(result, Some(0));
    }

    #[test]
    fn test_brute_force_rotation_nonzero() {
        let strings = vec!["10".into(), "20".into(), "30".into(), "40".into()];
        let terms = vec![
            ChecksumTerm {
                indices: vec![0x100],
                divisors: vec![1.0],
                sign: 1,
            },
            ChecksumTerm {
                indices: vec![0x101],
                divisors: vec![2.0],
                sign: 1,
            },
        ];

        let result = brute_force_rotation(&strings, &terms, 35.0, 0x100, OffsetOperation::Subtract);
        assert_eq!(result, Some(1));
    }
}
