#[cfg(test)]
use super::*;
use crate::ast_deobfuscate::DeobfuscateState;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_elimination(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut collector = DeadVarCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let dead_vars = collector.get_dead_vars();
    eprintln!("Dead vars: {dead_vars:?}");

    if !dead_vars.is_empty() {
        let mut eliminator = DeadVarEliminator::new(dead_vars);
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut eliminator, &mut program, &mut ctx);
    }

    Codegen::new().build(&program).code
}

#[test]
fn test_remove_unused_var() {
    let output = run_elimination("var unused = 5;");
    eprintln!("Output: {output}");
    assert!(
        !output.contains("unused"),
        "Should remove unused variable, got: {output}"
    );
}

#[test]
fn test_preserve_used_var() {
    let output = run_elimination("var used = 5; console.log(used);");
    assert!(output.contains("used"), "Should preserve used variable, got: {output}");
}

#[test]
fn test_remove_multiple_unused() {
    let output = run_elimination("var a = 1; var b = 2;");
    assert!(
        !output.contains('a') && !output.contains('b'),
        "Should remove multiple unused variables, got: {output}"
    );
}

#[test]
fn test_preserve_with_side_effects() {
    let output = run_elimination("var x = someFunction();");
    assert!(
        output.contains("someFunction"),
        "Should preserve declaration with side effects, got: {output}"
    );
}

#[test]
fn test_preserve_function_params() {
    let output = run_elimination("function f(param) { return 1; }");
    assert!(
        output.contains("param"),
        "Should preserve function parameters, got: {output}"
    );
}

#[test]
fn test_mixed_declarations() {
    let output = run_elimination("var used = 1, unused = 2; console.log(used);");
    eprintln!("Mixed output: {output}");
    assert!(output.contains("used"), "Should preserve used variable, got: {output}");
}

#[test]
fn test_written_but_not_read() {
    let output = run_elimination("var x = 1; x = 2;");
    eprintln!("Written not read output: {output}");
    assert!(
        !output.contains("var x"),
        "Should remove variable that is only written, got: {output}"
    );
}

#[test]
fn test_preserve_read_after_write() {
    let output = run_elimination("var x = 1; x = 2; console.log(x);");
    assert!(
        output.contains("var x"),
        "Should preserve variable that is read, got: {output}"
    );
}
