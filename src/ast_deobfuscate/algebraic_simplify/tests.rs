use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, semantic};
#[cfg(test)]
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run_simplify(code: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut simplifier = AlgebraicSimplifier::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

    traverse_mut_with_ctx(&mut simplifier, &mut program, &mut ctx);

    Codegen::new().build(&program).code
}

#[test]
fn test_literal_left_operand() {
    assert_eq!(run_simplify("r = true && x;"), "r = x;\n");
    assert_eq!(run_simplify("r = false && x;"), "r = false;\n");
    assert_eq!(run_simplify("r = true || x;"), "r = true;\n");
    assert_eq!(run_simplify("r = false || x;"), "r = x;\n");
}

#[test]
fn test_literal_right_operand_kept() {
    for code in [
        "r = x && true;\n",
        "r = x && false;\n",
        "r = x || true;\n",
        "r = x || false;\n",
        "r = foo() || true;\n",
    ] {
        assert_eq!(run_simplify(code), code);
    }
}

#[test]
fn test_arithmetic_identities_kept() {
    for code in [
        "r = [b] + 0;\n",
        "r = 0 + [b];\n",
        "r = y / 1;\n",
        "r = y * 1;\n",
        "r = f() * 0;\n",
        "r = x - x;\n",
        "r = x / x;\n",
    ] {
        assert_eq!(run_simplify(code), code);
    }
}
