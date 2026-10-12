use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_rename(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut renamer = DeterministicRenamer::new();
    renamer.analyze_source(code);
    renamer.build_rename_map(code);

    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn test_slot_based_rename() {
    let code = "var q = 1; var K = 2; console.log(q, K);";
    let output = run_rename(code);

    assert!(
        output.contains("s0") || output.contains("s1"),
        "Should have slot-based names"
    );
    assert!(output.contains("console"), "Should preserve globals");
}

#[test]
fn test_preserves_globals() {
    let code = "console.log(JSON.stringify({}));";
    let output = run_rename(code);

    assert!(output.contains("console"));
    assert!(output.contains("JSON"));
    assert!(output.contains("stringify"));
}

#[test]
fn test_preserves_keywords() {
    let code = "function test() { return true; }";
    let output = run_rename(code);

    assert!(output.contains("function"));
    assert!(output.contains("return"));
    assert!(output.contains("true"));
}

#[test]
fn test_skip_0x_prefix() {
    let code = "var _0x1234 = 42;";
    let output = run_rename(code);

    assert!(output.contains("_0x1234"));
}

#[test]
fn test_consistent_slots() {
    let code1 = "var q = 1; var K = 2;";
    let code2 = "var q = 1; var K = 2;";

    let output1 = run_rename(code1);
    let output2 = run_rename(code2);

    assert_eq!(output1, output2, "Same code should produce same output");
}
