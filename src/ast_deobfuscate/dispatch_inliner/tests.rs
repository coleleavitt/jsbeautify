use super::*;
use crate::ast_deobfuscate::semantic;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;

    let mut collector = DispatchInlinerCollector::new();
    let state = DeobfuscateState::new();
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

    let (factories, constants) = collector.into_maps();
    if !factories.is_empty() {
        let mut rewriter = DispatchInlinerRewriter::new(factories, constants);
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut rewriter, &mut program, &mut ctx);
    }

    Codegen::new().build(&program).code
}

#[test]
fn resolves_simple_array_factory() {
    let out = run(r#"function f(){return ["a","b","c"];} var r = f()[1];"#);
    assert!(out.contains("\"b\""), "got: {out}");
    assert!(!out.contains("f()[1]"), "call site should be replaced: {out}");
}

#[test]
fn resolves_self_init_accessor_array() {
    let code = r#"
            function f() {
                var a = ["x", "y"];
                f = function() { return a; };
                return a;
            }
            var r = f()[0];
        "#;
    let out = run(code);
    assert!(out.contains("\"x\""), "got: {out}");
    assert!(!out.contains("f()[0]"), "call should be replaced: {out}");
}

#[test]
fn resolves_with_index_constant() {
    let code = r#"
            var I = 2;
            function f(){return ["a","b","c"];}
            var r = f()[I];
        "#;
    let out = run(code);
    assert!(out.contains("\"c\""), "got: {out}");
}

#[test]
fn preserves_non_constant_index() {
    let code = r#"function f(){return ["a","b"];} var r = f()[x];"#;
    let out = run(code);
    assert!(out.contains("f()[x]"), "non-constant index must be preserved: {out}");
}

#[test]
fn preserves_non_array_factory() {
    let code = r"function f(){return compute();} var r = f()[0];";
    let out = run(code);
    assert!(out.contains("f()[0]"), "non-array factory must be preserved: {out}");
}

#[test]
fn handles_large_array() {
    let mut elements: Vec<String> = Vec::new();
    for i in 0..150 {
        elements.push(format!("\"s{i}\""));
    }
    let array_str = elements.join(",");
    let code = format!("function f(){{return [{array_str}];}} var r = f()[149];");
    let out = run(&code);
    assert!(out.contains("\"s149\""), "got: {out}");
}

#[test]
fn resolves_multiple_factories() {
    let code = r#"
            function x19(){return ["BR","t5","B5"];}
            function Y49(){return ["HR","tg","gX"];}
            var a = x19()[0];
            var b = Y49()[2];
        "#;
    let out = run(code);
    assert!(out.contains("\"BR\""), "x19()[0] should resolve: {out}");
    assert!(out.contains("\"gX\""), "Y49()[2] should resolve: {out}");
}

#[test]
fn out_of_bounds_index_preserved() {
    let code = r#"function f(){return ["a","b"];} var r = f()[5];"#;
    let out = run(code);
    assert!(out.contains("f()[5]"), "out-of-bounds must be preserved: {out}");
}

#[test]
fn ignores_factory_with_args() {
    let code = r#"function f(){return ["a","b"];} var r = f(1)[0];"#;
    let out = run(code);
    assert!(out.contains("f(1)"), "call with args must be preserved: {out}");
}

#[test]
fn resolves_bare_assignment_constant() {
    let code = r#"NF = 0; function f(){return ["a","b"];} var r = f()[NF];"#;
    let out = run(code);
    assert!(out.contains("\"a\""), "bare assignment constant should resolve: {out}");
    assert!(!out.contains("f()[NF]"), "call site should be replaced: {out}");
}

#[test]
fn resolves_mixed_var_and_bare() {
    let code = r#"var I = 1; J = 2; function f(){return ["a","b","c"];} var r = f()[J];"#;
    let out = run(code);
    assert!(out.contains("\"c\""), "bare assignment constant should resolve: {out}");
    assert!(!out.contains("f()[J]"), "call site should be replaced: {out}");
}

#[test]
fn resolves_computed_constant() {
    let code = r#"var A = 1; var B = A + A; function f(){return ["x","y","z"];} var r = f()[B];"#;
    let out = run(code);
    assert!(
        out.contains("\"z\""),
        "computed constant B=2 should resolve to index 2: {out}"
    );
    assert!(!out.contains("f()[B]"), "call site should be replaced: {out}");
}

#[test]
fn resolves_chain() {
    let code = r#"X = 2; Y = X * 3; Z = Y - 1; function f(){return ["a","b","c","d","e","f"];} var r = f()[Z];"#;
    let out = run(code);
    assert!(
        out.contains("\"f\""),
        "chained constant Z=5 should resolve to index 5: {out}"
    );
    assert!(!out.contains("f()[Z]"), "call site should be replaced: {out}");
}

#[test]
fn detects_flattened_factory() {
    let code = r#"var __cache = ["a","b","c"]; function F() { return __cache; } var r = F()[1];"#;
    let out = run(code);
    assert!(out.contains("\"b\""), "got: {out}");
    assert!(!out.contains("F()[1]"), "call site should be replaced: {out}");
}

#[test]
fn detects_flattened_with_constant_index() {
    let code = r#"var I = 2; var __cache = ["x","y","z"]; function F() { return __cache; } var r = F()[I];"#;
    let out = run(code);
    assert!(out.contains("\"z\""), "got: {out}");
    assert!(!out.contains("F()[I]"), "call site should be replaced: {out}");
}

#[test]
fn preserves_non_array_return() {
    let code = r"var __cache = compute(); function F() { return __cache; } var r = F()[0];";
    let out = run(code);
    assert!(out.contains("F()[0]"), "non-array return must be preserved: {out}");
}
