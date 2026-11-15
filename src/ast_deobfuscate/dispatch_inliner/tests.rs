use super::*;
use crate::ast_deobfuscate::semantic;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    run_as(code, SourceType::mjs())
}

/// Runs the pass on sloppy-mode script input.
fn run_script(code: &str) -> String {
    run_as(code, SourceType::cjs())
}

fn run_as(code: &str, source_type: SourceType) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, source_type).parse();
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

/// Inputs whose `f()[k]` reads must stay: the factory or the index binding a
/// call site names is not (only) the one its definition gives a value.
#[test]
fn keeps_call_sites_a_binding_may_not_reach() {
    for code in [
        // A parameter or a catch binding shadows the factory or constant.
        r#"function f(){return ["a","b","c"];} function g(f){ return f()[1]; }"#,
        r#"var I = 0; function f(){return ["a","b","c"];} try { throw 2; } catch (I) { f()[I]; }"#,
        r#"var I = 0; function f(){return ["a","b","c"];} function g(I){ return f()[I]; }"#,
        // A block's `let` is not a body-level definition.
        r#"function f(){return ["a","b","c"];} { let K = 1; f()[K]; }"#,
        // Written after its definition.
        r#"var J = 0; function f(){return ["a","b","c"];} J = 2; f()[J];"#,
        r#"var J = 0; function f(){return ["a","b","c"];} J++; f()[J];"#,
        r#"function f(){return ["a","b"];} f = function(){return ["x","y"];}; f()[0];"#,
        r#"var c = ["a","b"]; function F(){return c;} c = ["x","y"]; F()[0];"#,
        r#"function f(){return ["a","b","c"];} N = 1; N += 1; f()[N];"#,
        // Read before its definition runs.
        r#"function f(){return ["a","b"];} f()[I]; var I = 1;"#,
        r#"function f(){return ["a","b"];} if (x) { var I = 1; } f()[I];"#,
        // A shared array that a caller mutates.
        r#"function f(){ var a = ["a","b"]; f = function(){return a;}; return a; } f().reverse(); f()[0];"#,
        r#"function f(){ var a = ["a","b"]; f = function(){return a;}; return a; } delete f()[0]; f()[1];"#,
        // An async factory returns a promise; a default parameter runs code.
        r#"async function f(){return ["a","b"];} f()[0];"#,
        r#"function f(x = n++){return ["a","b"];} f()[0];"#,
    ] {
        let out = run(code);
        assert!(out.contains("()["), "a call site was inlined in {code}:\n{out}");
    }
}

#[test]
fn keeps_call_sites_a_with_body_or_eval_may_redirect() {
    for code in [
        r#"function f(){return ["a","b"];} var o = {I: 5}; with (o) { var I = 1; } f()[I];"#,
        r#"function f(){return ["a","b"];} var o = {}; with (o) { N = 1; } N = 0; f()[N];"#,
        r#"function f(){return ["a","b"];} var I = 0; eval("I = 1"); f()[I];"#,
        r#"function f(){return ["a","b"];} function g(){ eval("var f"); return f()[0]; }"#,
        r#"function f(){return ["a","b"];} N = 0; (0, function(){ (eval)("N = 1"); })(); f()[N];"#,
    ] {
        let out = run_script(code);
        assert!(out.contains("()["), "a call site was inlined in {code}:\n{out}");
    }
}

#[test]
fn keeps_call_sites_an_annex_b_function_may_redirect() {
    let code = r#"{ function f(){return ["x","y"];} } f()[0]; function f(){return ["a","b"];}"#;
    let out = run_script(code);
    assert!(out.contains("f()[0]"), "got: {out}");
}

#[test]
fn resolves_through_unrelated_same_named_bindings() {
    let code = r#"
            var I = 1;
            function f(){return ["a","b","c"];}
            function g(I, f){ return I + f; }
            var r = f()[I];
        "#;
    let out = run(code);
    assert!(out.contains(r#"r = "b""#), "got: {out}");
}

#[test]
fn resolves_each_call_site_to_its_own_factory() {
    let code = r#"function f(){return ["a","b"];} function g(){ function f(){return ["x","y"];} return f()[1]; } var r = f()[1];"#;
    let out = run(code);
    assert!(
        out.contains(r#"return "y""#) && out.contains(r#"r = "b""#),
        "got: {out}"
    );
}

#[test]
fn resolves_shared_array_factories_read_only_by_index() {
    let code = r#"var c = ["a","b"]; function F(){return c;} var r = F()[1] + F()[0];"#;
    let out = run(code);
    assert!(out.contains(r#""b" + "a""#), "got: {out}");
}

#[test]
fn keeps_classic_script_globals_other_scripts_may_change() {
    for code in [
        r#"function f(){return ["a","b"];} var r = f()[0];"#,
        r#"var I = 1; (function(){ function f(){return ["a","b"];} r = f()[I]; })();"#,
        r#"NF = 1; (function(){ function f(){return ["a","b"];} r = f()[NF]; })();"#,
    ] {
        let out = run_as(code, SourceType::script());
        assert!(out.contains("()["), "a call site was inlined in {code}:\n{out}");
    }
    let code = r#"(function(){ var I = 1; function f(){return ["a","b"];} r = f()[I]; })();"#;
    let out = run_as(code, SourceType::script());
    assert!(out.contains(r#"r = "b""#), "got: {out}");
}

/// A function declaration is hoisted: a call above a constant's definition
/// runs its body before the definition.
#[test]
fn keeps_reads_a_hoisted_call_may_run_before_the_definition() {
    for code in [
        r#"function f(){return ["a","b","c"];} g(); var I = 1; function g(){ return f()[I]; }"#,
        r#"function f(){return ["a","b","c"];} g(); N = 1; function g(){ return f()[N]; }"#,
        r#"function f(){return ["a","b","c"];} function outer(){ inner(); var I = 1; function inner(){ return f()[I]; } }"#,
        r#"g(); var c = ["a","b"]; function F(){return c;} function g(){ return F()[0]; }"#,
        r#"import "m"; function f(){return ["a","b","c"];} var I = 1; export function g(){ return f()[I]; }"#,
    ] {
        let out = run(code);
        assert!(out.contains("()["), "a call site was inlined in {code}:\n{out}");
    }
}

#[test]
fn resolves_hoisted_reads_of_definitions_no_code_runs_before() {
    let code = r#"
            var A = 1, s = "x";
            function f(){return ["a","b","c"];}
            var B = A + 1;
            function g(){ return f()[B]; }
            g();
            function outer(){ var I = 1; var h = () => f()[I]; return h(); }
        "#;
    let out = run(code);
    assert!(out.contains(r#"return "c""#), "got: {out}");
    assert!(out.contains(r#"=> "b""#), "got: {out}");
}

/// Fixes from review: each input's `f()[k]` must stay.
#[test]
fn keeps_reads_code_may_reach_before_or_around_the_definition() {
    for code in [
        // A re-exported module runs first and may call the hoisted `h`.
        r#"function f(){return ["a","b","c","d"];} var J = 1; function h(){ return f()[J]; } export { h }; export * from "./b.mjs";"#,
        r#"function f(){return ["a","b","c","d"];} var J = 1; function h(){ return f()[J]; } export { h }; export { x } from "./b.mjs";"#,
        // `g()` runs before `J = 1` in the same declaration.
        r#"function f(){return ["a","b","c"];} var K = g(), J = 1; function g(){ return f()[J]; }"#,
        // A shared array read through a non-index key reaches `Array.prototype` accessors.
        r#"function f(){ var a = ["a","b"]; f = function(){return a;}; return a; } var K = "x"; f()[K]; var r = f()[0];"#,
        // Past 2^53, doubles round: `A + 1 - A` is 0 in JS.
        r#"function f(){return ["a","b"];} var A = 9007199254740992; var B = A + 1 - 9007199254740992; f()[B];"#,
        r#"function f(){return ["a","b"];} var A = 4503599627370496; var B = A * 2 + 1 - A * 2; f()[B];"#,
    ] {
        let out = run(code);
        assert!(out.contains("()["), "a call site was inlined in {code}:\n{out}");
    }
    for code in [
        // Sloppy-mode `arguments[0]` writes the parameter.
        r#"function f(){return ["a","b","c"];} function g(J){ J = 0; arguments[0] = 2; return f()[J]; }"#,
        // The global object or global-scope code may assign the global.
        r#"function f(){return ["a","b"];} N = 0; globalThis.N = 1; f()[N];"#,
        r#"function f(){return ["a","b"];} N = 0; (0, eval)("N = 1"); f()[N];"#,
        r#"function f(){return ["a","b"];} N = 0; Function("N = 1")(); f()[N];"#,
    ] {
        let out = run_script(code);
        assert!(out.contains("()["), "a call site was inlined in {code}:\n{out}");
    }
}
