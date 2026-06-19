#[cfg(test)]
use super::*;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> String {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;
    let mut collector = LookupForwarderCollector::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
    traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);
    let fwds = collector.forwarders();
    if !fwds.is_empty() {
        let mut inliner = LookupForwarderInliner::new(fwds);
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);
    }
    Codegen::new().build(&program).code
}

#[test]
fn inlines_fn_declaration_forwarder() {
    let out = run("function bI(x) { return x19()[x]; } var r = bI(5);");
    assert!(out.contains("x19()[5]"), "got: {out}");
    assert!(!out.contains("function bI"), "forwarder must be removed: {out}");
}

#[test]
fn inlines_var_forwarder() {
    let out = run("var bI = function(x) { return x19()[x]; }; var r = bI(y);");
    assert!(out.contains("x19()[y]"), "got: {out}");
    assert!(!out.contains("var bI"), "declaration must be removed: {out}");
}

#[test]
fn leaves_multi_arg_alone() {
    let out = run("function f(x, y) { return x19()[x]; } f(1, 2);");
    assert!(out.contains("function f"), "two-arg fn must be kept: {out}");
}

#[test]
fn leaves_non_member_return_alone() {
    let out = run("function f(x) { return x + 1; } f(5);");
    assert!(out.contains("function f"), "must keep: {out}");
}

#[test]
fn leaves_accessor_with_args_alone() {
    let out = run("function f(x) { return acc(1)[x]; } f(5);");
    assert!(out.contains("function f"), "must keep (accessor takes args): {out}");
}

#[test]
fn does_not_remove_same_name_var_with_different_shape() {
    // Inner function Xt is a lookup forwarder; outer var Xt is a 2-arg operator proxy.
    // The inliner must NOT remove the outer var Xt declaration.
    let code = r"
            var Xt = function(a, b) { return a + b; };
            (function Ql() {
                function Xt(x) { return Tl()[x]; }
                var r = Xt(5);
            })();
            var z = Xt(1, 2);
        ";
    let out = run(code);
    assert!(
        out.contains("var Xt"),
        "outer var Xt (operator proxy) must be preserved: {out}"
    );
}
