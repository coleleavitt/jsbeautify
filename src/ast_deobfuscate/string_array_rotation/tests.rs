use super::*;
use crate::ast_deobfuscate::semantic;
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn run_string_array_rotation(code: &str) -> (String, StringArrayRotation, DeobfuscateState) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut pass = StringArrayRotation::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);

    oxc_traverse::traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);

    let mut state = DeobfuscateState::new();
    pass.finalize(&mut state);

    let output = Codegen::new().build(&program).code;
    (output, pass, state)
}

#[test]
fn test_detect_string_array() {
    let code = r#"var _0x1234 = ["hello", "world", "test"];"#;

    let (_, _pass, state) = run_string_array_rotation(code);

    assert_eq!(state.string_arrays.len(), 1);
    assert!(state.string_arrays.contains_key("_0x1234"));

    let array = &state.string_arrays["_0x1234"];
    assert_eq!(array.strings, vec!["hello", "world", "test"]);
    assert!(!array.rotated);
}

#[test]
fn test_rotation() {
    let code = r#"
            var _0x1111 = ["a", "b", "c"];
            (function(_0x2222, _0x3333) {
                var _0x4444 = function(_0x5555) {
                    while (--_0x5555) {
                        _0x2222.push(_0x2222.shift());
                    }
                };
                _0x4444(2);
            })(_0x1111, 0x123);
        "#;

    let (_, _pass, state) = run_string_array_rotation(code);

    assert_eq!(state.string_arrays.len(), 1);
    assert!(state.string_arrays.contains_key("_0x1111"));

    let array = &state.string_arrays["_0x1111"];
    assert!(array.rotated, "Array should be marked as rotated");
    assert_eq!(array.rotation_count, 2);
    assert_eq!(
        array.strings,
        vec!["c", "a", "b"],
        "Array should be rotated by 2 positions"
    );
}

#[test]
fn test_apply_rotation() {
    let mut strings = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    StringArrayRotation::apply_rotation(&mut strings, 1);
    assert_eq!(strings, vec!["b", "c", "a"]);

    let mut strings2 = vec!["x".to_string(), "y".to_string(), "z".to_string()];
    StringArrayRotation::apply_rotation(&mut strings2, 2);
    assert_eq!(strings2, vec!["z", "x", "y"]);
}
