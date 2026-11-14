use super::*;
use crate::ast_deobfuscate::{AstDeobfuscator, semantic};
use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Statement};
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use rustc_hash::FxHashSet;

struct EliminationResult {
    output: String,
    candidate_symbols: usize,
    removed_declarators: usize,
}

fn run_elimination(code: &str) -> EliminationResult {
    run_elimination_as(code, SourceType::mjs())
}

fn run_elimination_as(code: &str, source_type: SourceType) -> EliminationResult {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, source_type).parse();
    assert!(
        ret.diagnostics.is_empty(),
        "test input failed to parse: {:?}",
        ret.diagnostics
    );
    let mut program = ret.program;

    let scoping = semantic::scoping(&program);
    let (result, _) = eliminate_dead_vars(&allocator, &mut program, scoping);

    EliminationResult {
        output: Codegen::new().build(&program).code,
        candidate_symbols: result.candidate_symbols(),
        removed_declarators: result.removed_declarators(),
    }
}

#[test]
fn removes_unused_pure_declaration_and_counts_rewrite() {
    let result = run_elimination("let unused = 5;");
    assert!(!result.output.contains("unused"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 1);
    assert_eq!(result.removed_declarators, 1);
}

#[test]
fn preserves_used_declaration() {
    let result = run_elimination("let used = 5; console.log(used);");
    assert!(result.output.contains("let used = 5"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn counts_only_declarators_actually_removed_from_mixed_declaration() {
    let result = run_elimination("let used = 1, unused = 2; consume(used);");
    assert!(result.output.contains("let used = 1"), "got: {}", result.output);
    assert!(!result.output.contains("unused"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 1);
    assert_eq!(result.removed_declarators, 1);
}

#[test]
fn repeated_var_symbol_reports_one_candidate_and_two_declarators() {
    let result = run_elimination("var repeated = 1; var repeated;");
    assert!(!result.output.contains("repeated"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 1);
    assert_eq!(result.removed_declarators, 2);
}

#[test]
fn direct_eval_disables_elimination_for_entire_program() {
    let result = run_elimination("let visible = 1; eval('visible');");
    assert!(result.output.contains("let visible = 1"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 0);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn parenthesized_direct_eval_disables_elimination() {
    let result = run_elimination("let visible = 1; (eval)('visible');");
    assert!(result.output.contains("let visible = 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn nested_eval_preserves_ancestor_binding() {
    let result = run_elimination(
        "function outer() { let ancestor = 1; function inner() { eval('ancestor'); } inner(); } outer();",
    );
    assert!(result.output.contains("let ancestor = 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn ancestor_eval_preserves_nested_binding_conservatively() {
    let result =
        run_elimination("function outer() { eval('nested'); function inner() { let nested = 1; } inner(); } outer();");
    assert!(result.output.contains("let nested = 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn shadowed_eval_name_still_fails_closed() {
    let result = run_elimination("function run(eval) { let hidden = 1; eval('hidden'); } run(callback);");
    assert!(result.output.contains("let hidden = 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_number_alias_read_on_assignment_rhs() {
    let result = run_elimination("let numberAlias = Number; assigned = numberAlias;");
    assert!(result.output.contains("numberAlias = Number"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn rejects_candidate_that_would_make_live_reference_unresolved() {
    let allocator = Allocator::default();
    let parsed = Parser::new(
        &allocator,
        "let liveBinding = 1; consume(liveBinding);",
        SourceType::mjs(),
    )
    .parse();
    assert_eq!(parsed.diagnostics.len(), 0);
    let mut program = parsed.program;
    let semantic = semantic::build(&program);
    assert_eq!(semantic.diagnostics.len(), 0);

    let Statement::VariableDeclaration(declaration) = &program.body[0] else {
        panic!("expected variable declaration");
    };
    let BindingPattern::BindingIdentifier(binding) = &declaration.declarations[0].id else {
        panic!("expected binding identifier");
    };
    let symbol_id = binding.symbol_id.get().expect("semantic symbol id");
    let result = super::sweep::eliminate_dead_vars_with_forced_candidates(
        &allocator,
        &mut program,
        FxHashSet::from_iter([symbol_id]),
    );
    let output = Codegen::new().build(&program).code;

    assert!(result.was_rejected());
    assert_eq!(result.candidate_symbols(), 1);
    assert_eq!(result.removed_declarators(), 0);
    assert!(output.contains("let liveBinding = 1"), "got: {output}");
    assert!(output.contains("consume(liveBinding)"), "got: {output}");
}

#[test]
fn known_globals_do_not_reject_valid_elimination() {
    let allocator = Allocator::default();
    let parsed = Parser::new(
        &allocator,
        "let unused = 1; console.log(Number, Worker, Uint8Array);",
        SourceType::mjs(),
    )
    .parse();
    assert_eq!(parsed.diagnostics.len(), 0);
    let mut program = parsed.program;
    let scoping = semantic::scoping(&program);
    let (result, _) = eliminate_dead_vars(&allocator, &mut program, scoping);
    let output = Codegen::new().build(&program).code;

    assert!(!result.was_rejected());
    assert_eq!(result.removed_declarators(), 1);
    assert!(!output.contains("unused"), "got: {output}");
    assert!(
        output.contains("console.log(Number, Worker, Uint8Array)"),
        "got: {output}"
    );
}

#[test]
fn normal_pipeline_preserves_assignment_rhs_alias() {
    let output = AstDeobfuscator::new()
        .with_skip_annotations(true)
        .deobfuscate("let numberAlias = Number; assigned = numberAlias;")
        .expect("pipeline should succeed");

    assert!(output.contains("numberAlias = Number"), "got: {output}");
    assert!(output.contains("assigned = numberAlias"), "got: {output}");
}

#[test]
fn preserves_conditional_worker_alias_read_on_assignment_rhs() {
    let result =
        run_elimination("let WorkerAlias = workerAvailable ? Worker : FallbackWorker; selectedWorker = WorkerAlias;");
    assert!(result.output.contains("let WorkerAlias"), "got: {}", result.output);
    assert!(
        result.output.contains("selectedWorker = WorkerAlias"),
        "got: {}",
        result.output
    );
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_conditional_uint8array_alias_read_on_assignment_rhs() {
    let result =
        run_elimination("let ByteArray = typedArraysAvailable ? Uint8Array : Array; selectedArray = ByteArray;");
    assert!(result.output.contains("let ByteArray"), "got: {}", result.output);
    assert!(
        result.output.contains("selectedArray = ByteArray"),
        "got: {}",
        result.output
    );
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_rhs_read_in_nested_assignments() {
    let result = run_elimination("let numberAlias = Number; outer = inner = numberAlias;");
    assert!(result.output.contains("numberAlias = Number"), "got: {}", result.output);
    assert!(
        result.output.contains("outer = inner = numberAlias"),
        "got: {}",
        result.output
    );
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn separates_shadowed_symbols() {
    let result =
        run_elimination("let value = 0; function choose() { let value = Number; assigned = value; } choose();");
    assert!(!result.output.contains("let value = 0"), "got: {}", result.output);
    assert!(result.output.contains("let value = Number"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 1);
}

#[test]
fn separates_same_name_bindings_in_nested_scopes() {
    let result = run_elimination(
        "function outer() { let alias = 1; function inner() { let alias = Number; assigned = alias; } inner(); } outer();",
    );
    assert!(!result.output.contains("let alias = 1"), "got: {}", result.output);
    assert!(result.output.contains("let alias = Number"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 1);
}

#[test]
fn preserves_write_only_declaration() {
    let result = run_elimination("let local = 0; local = 1;");
    assert!(result.output.contains("let local = 0"), "got: {}", result.output);
    assert!(result.output.contains("local = 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_update_target_as_read_and_write() {
    let result = run_elimination("let counter = 0; counter++;");
    assert!(result.output.contains("let counter = 0"), "got: {}", result.output);
    assert!(result.output.contains("counter++"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_rhs_read_inside_destructuring_assignment() {
    let result = run_elimination("let numberAlias = Number; ({ value: result } = { value: numberAlias });");
    assert!(result.output.contains("numberAlias = Number"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn fails_closed_for_unsupported_destructuring_declaration() {
    let result = run_elimination("const { value } = source;");
    assert!(
        result.output.contains("const { value } = source"),
        "got: {}",
        result.output
    );
    assert_eq!(result.candidate_symbols, 0);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_side_effectful_initializer_and_reports_no_rewrite() {
    let result = run_elimination("let unused = effect();");
    assert!(result.output.contains("effect()"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 1);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn for_head_declarator_is_a_candidate_but_not_removed() {
    let result = run_elimination("for (var unused = 0; ; ) { break; }");
    assert!(result.output.contains("unused = 0"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 1);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn removes_pure_declarator_nested_in_a_function_beside_an_unremovable_one() {
    let result = run_elimination("let kept = effect(); export function f() { if (x) { let unused = 1; } }");
    assert!(!result.output.contains("unused"), "got: {}", result.output);
    assert!(result.output.contains("kept = effect()"), "got: {}", result.output);
    assert_eq!(result.candidate_symbols, 2);
    assert_eq!(result.removed_declarators, 1);
}

#[test]
fn returns_the_scoping_only_when_the_program_is_unchanged() {
    let allocator = Allocator::default();
    let source = "let kept = effect(); let used = 1; consume(used);";
    let mut program = Parser::new(&allocator, source, SourceType::mjs()).parse().program;
    let scoping = semantic::scoping(&program);
    let (result, scoping) = eliminate_dead_vars(&allocator, &mut program, scoping);
    assert_eq!(result.removed_declarators(), 0);
    let returned = scoping.expect("an unchanged program keeps its scoping");
    let fresh = semantic::scoping(&program);
    assert_eq!(
        returned.symbol_names().collect::<Vec<_>>(),
        fresh.symbol_names().collect::<Vec<_>>()
    );

    let mut program = Parser::new(&allocator, "let unused = 1;", SourceType::mjs())
        .parse()
        .program;
    let scoping = semantic::scoping(&program);
    let (result, scoping) = eliminate_dead_vars(&allocator, &mut program, scoping);
    assert_eq!(result.removed_declarators(), 1);
    assert!(scoping.is_none());
}

#[test]
fn preserves_missing_identifier_initializer() {
    let result = run_elimination("let unused = definitelyMissing;");
    assert!(result.output.contains("definitelyMissing"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_tdz_initializer() {
    let result = run_elimination("let unused = later; let later = 1;");
    assert!(result.output.contains("let unused = later"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_instanceof_that_can_throw_type_error() {
    let result = run_elimination("let unused = value instanceof 1;");
    assert!(result.output.contains("instanceof 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_array_and_object_spread_initializers() {
    let array = run_elimination("let unused = [...iterable];");
    assert!(array.output.contains("...iterable"), "got: {}", array.output);
    assert_eq!(array.removed_declarators, 0);

    let object = run_elimination("let unused = { ...source };");
    assert!(object.output.contains("...source"), "got: {}", object.output);
    assert_eq!(object.removed_declarators, 0);
}

#[test]
fn preserves_symbol_to_primitive_coercion() {
    let result = run_elimination(
        "let coercive = { [Symbol.toPrimitive]() { throw new Error('coerced'); } }; let unused = coercive + 1; consume(coercive);",
    );
    assert!(result.output.contains("Symbol.toPrimitive"), "got: {}", result.output);
    assert!(result.output.contains("coercive + 1"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_this_before_super_in_derived_constructor() {
    let result =
        run_elimination("class Child extends Base { constructor() { let unused = this; super(); } } consume(Child);");
    assert!(result.output.contains("let unused = this"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_property_access_initializer_as_getter_guard() {
    let result = run_elimination("let unused = source.value;");
    assert!(result.output.contains("source.value"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn preserves_exported_declarations() {
    let direct = run_elimination("export const publicValue = 1;");
    assert!(direct.output.contains("publicValue"), "got: {}", direct.output);
    assert_eq!(direct.removed_declarators, 0);

    let specifier = run_elimination("const publicValue = 1; export { publicValue };");
    assert!(specifier.output.contains("publicValue"), "got: {}", specifier.output);
    assert_eq!(specifier.removed_declarators, 0);
}

#[test]
fn script_keeps_unused_globals_but_sweeps_locals() {
    let result = run_elimination_as(
        "var g = 1; let l = 2; if (g) { var nested = 3; } function f() { var local = 4; }",
        SourceType::script(),
    );
    for kept in ["var g = 1", "let l = 2", "var nested = 3"] {
        assert!(result.output.contains(kept), "{kept} is a global: {}", result.output);
    }
    assert!(!result.output.contains("local"), "got: {}", result.output);
}

#[test]
fn commonjs_sweeps_unused_top_level() {
    let result = run_elimination_as("var g = 1; return;", SourceType::cjs());
    assert!(!result.output.contains("var g"), "got: {}", result.output);
}

fn run_cjs(code: &str) -> EliminationResult {
    run_elimination_as(code, SourceType::cjs())
}

#[test]
fn removes_uncalled_decoder_together_with_its_string_array() {
    let result = run_elimination(
        r#"var strings = ["Hello", "log"]; function decode(i) { i = i - 100; return strings[i]; } console.log("Hello");"#,
    );
    assert!(!result.output.contains("strings"), "got: {}", result.output);
    assert!(!result.output.contains("decode"), "got: {}", result.output);
    assert!(
        result.output.contains(r#"console.log("Hello")"#),
        "got: {}",
        result.output
    );
    assert_eq!(result.candidate_symbols, 2);
    assert_eq!(result.removed_declarators, 2);
}

#[test]
fn keeps_decoder_and_array_reached_from_live_code() {
    let code =
        "var strings = ['a']; function decode(i) { return strings[i]; } function main() { return decode(0); } main();";
    let result = run_elimination(code);
    for kept in ["strings = [", "function decode", "function main"] {
        assert!(result.output.contains(kept), "{kept}: {}", result.output);
    }
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn removes_mutually_recursive_dead_functions() {
    let result = run_elimination("function a() { return b(); } function b() { return a(); } console.log(1);");
    assert!(!result.output.contains("function"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 2);
}

#[test]
fn self_reassignment_inside_its_own_body_does_not_keep_a_function() {
    let result = run_cjs(
        "function lazy() { const a = ['x']; lazy = function () { return a; }; return lazy(); } console.log(1);",
    );
    assert!(!result.output.contains("lazy"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 2);
}

#[test]
fn reassignment_from_outside_keeps_a_function() {
    let result = run_cjs("function f() {} f = 1;");
    assert!(result.output.contains("function f()"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn script_keeps_top_level_functions_but_sweeps_nested_ones() {
    let result = run_elimination_as(
        "var strings = ['a']; function decode() { return strings[0]; } function outer() { function inner() {} return 1; } outer();",
        SourceType::script(),
    );
    for kept in ["var strings", "function decode", "function outer"] {
        assert!(result.output.contains(kept), "{kept} is a global: {}", result.output);
    }
    assert!(!result.output.contains("inner"), "got: {}", result.output);
}

#[test]
fn with_statement_disables_elimination() {
    let result = run_cjs("var unused = 1; function dead() {} with (o) { y; }");
    assert!(result.output.contains("var unused = 1"), "got: {}", result.output);
    assert!(result.output.contains("function dead"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn keeps_declarations_that_rebind_a_parameter() {
    let result = run_cjs(
        "function g(p, q) { function p() {} var q = 2; return arguments[0] + arguments[1]; } console.log(g(1, 1));",
    );
    assert!(result.output.contains("function p()"), "got: {}", result.output);
    assert!(result.output.contains("var q = 2"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn keeps_annex_b_block_functions() {
    let result = run_cjs("function g() { { function h() {} } } g();");
    assert!(result.output.contains("function h()"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn keeps_exported_functions() {
    for code in [
        "export function f() {}",
        "function f() {} export { f };",
        "function f() {} export default f;",
        "export default function f() {}",
    ] {
        let result = run_elimination(code);
        assert!(result.output.contains("function f()"), "{code}: {}", result.output);
        assert_eq!(result.removed_declarators, 0, "{code}");
    }
}

#[test]
fn removes_side_effect_free_compound_initializers() {
    let result = run_elimination(
        "let a = [1, , 'x', [null]], b = { k: 1, 'q': () => g(), m() { return h(); }, get v() { return 1; } }, c = -1, d = `t`, e = /r/g, f = function () { return g(); };",
    );
    assert!(!result.output.contains("let"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 6);
}

#[test]
fn keeps_initializers_that_can_run_code_or_throw() {
    for code in [
        "let unused = { [key]: 1 };",
        "let unused = [g()];",
        "let unused = `${value}`;",
        "let unused = +1n;",
        "let unused = class { static { g(); } };",
    ] {
        let result = run_elimination(code);
        assert!(result.output.contains("unused"), "{code}: {}", result.output);
        assert_eq!(result.removed_declarators, 0, "{code}");
    }
}

#[test]
fn keeps_function_reached_from_an_unremovable_initializer() {
    let result = run_elimination("function helper() {} let unused = effect(helper);");
    assert!(result.output.contains("function helper"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}

#[test]
fn keeps_every_declaration_of_an_exported_binding() {
    for code in [
        "export var x = 1; var x = function () { return 2; };",
        "export var x = 1; if (c) { var x = [function () {}]; }",
    ] {
        let result = run_elimination(code);
        assert!(result.output.contains("function"), "{code}: {}", result.output);
        assert_eq!(result.removed_declarators, 0, "{code}");
    }
}

#[test]
fn keeps_bindings_named_like_a_block_function() {
    // Annex B does not hoist `f` past the block's `class f`, so `f()` reads the
    // outer `var f`; `oxc_semantic` resolves it to the block function.
    let result = run_cjs(
        "var f = function () { return 'outer'; }; function g() { { class f {} { function f() { return 'inner'; } } } return f(); } console.log(g());",
    );
    assert!(result.output.contains("var f = function"), "got: {}", result.output);
    assert_eq!(result.removed_declarators, 0);
}
