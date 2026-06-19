#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_consolidation(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut consolidator = ObjectSparsingConsolidator::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut consolidator, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn test_consolidate_simple() {
    let output = run_consolidation(
        r"
            var obj = {};
            obj.a = 1;
            obj.b = 2;
        ",
    );
    eprintln!("Output: {output}");
    assert!(
        output.contains("a:") && output.contains("b:"),
        "Expected consolidated object, got: {output}"
    );
}

#[test]
fn test_consolidate_with_strings() {
    let output = run_consolidation(
        r#"
            var config = {};
            config.name = "test";
            config.value = 42;
        "#,
    );
    eprintln!("Output: {output}");
    assert!(
        output.contains("name:") && output.contains("value:"),
        "Expected consolidated object, got: {output}"
    );
}

#[test]
fn test_no_consolidation_non_empty() {
    let output = run_consolidation(
        r"
            var obj = {existing: true};
            obj.a = 1;
        ",
    );
    eprintln!("Output: {output}");
    assert!(
        output.contains("obj.a = 1"),
        "Should preserve non-empty object assignment, got: {output}"
    );
}

#[test]
fn test_no_consolidation_gap() {
    let output = run_consolidation(
        r#"
            var obj = {};
            console.log("break");
            obj.a = 1;
        "#,
    );
    eprintln!("Output: {output}");
    assert!(
        output.contains("obj.a = 1"),
        "Should preserve assignment after gap, got: {output}"
    );
}
