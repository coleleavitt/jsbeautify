//! CFF (Control-Flow-Flattening) unflattener.
//!
//! Consumes a [`DispatcherMap`] (from [`super::dispatcher_detector`]) and inlines
//! dispatcher call sites by replacing them with the cloned case body.
//!
//! A call site `D(LABEL, args)` or `D.call(this, LABEL, args)` is inlined only
//! when `D` and `LABEL` resolve to the dispatcher and the case label, every
//! other free name in the case body resolves to the same binding at the call
//! site, and the body neither reads a context value the call site would rebind
//! nor leaves the case with `break`/`continue`. A statement-level call whose
//! body has no `return`, declaration or args-parameter use is spliced as bare
//! statements; every other call becomes an arrow IIFE, which keeps the call
//! site's `this` and binds the args parameter locally.

use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::Statement;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;

/// The statements one dispatcher call runs for a case label.
pub struct CaseBody<'a> {
    /// The dispatcher's binding.
    dispatcher: Option<SymbolId>,
    /// The case test's reference to the label.
    label: ReferenceId,
    stmts: OxcVec<'a, Statement<'a>>,
}

type CaseBodyMap<'a> = FxHashMap<(String, String), CaseBody<'a>>;

mod case_bodies;
mod unflattener;

pub use case_bodies::collect_case_bodies;
pub use unflattener::CffUnflattener;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::dispatcher_detector::DispatcherDetector;
    use crate::ast_deobfuscate::semantic;
    use crate::ast_deobfuscate::state::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let detector = DispatcherDetector::new();
        let dispatchers = detector.detect(&program);
        if dispatchers.is_empty() {
            return (Codegen::new().build(&program).code, 0);
        }

        let scoping = semantic::scoping(&program);
        let case_bodies = collect_case_bodies(&program, &dispatchers, &allocator);
        let mut unflattener = CffUnflattener::new(dispatchers, case_bodies);

        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut unflattener, &mut program, &mut ctx);

        let inlined = unflattener.inlined();
        (Codegen::new().build(&program).code, inlined)
    }

    #[test]
    fn inlines_simple_dispatcher_call() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0] + 1; } break;
                }
            }
            var r = D(X, [5]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline 1 call, got: {out}");
        assert!(
            out.contains("var a = [5]"),
            "should have var decl for args param: {out}"
        );
        assert!(
            !out.contains("D(X,") && !out.contains("D(X, "),
            "original call should be replaced: {out}"
        );
    }

    #[test]
    fn preserves_non_literal_state() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0]; } break;
                }
            }
            D(someVar, [5]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 0, "should not inline computed state: {out}");
        assert!(out.contains("D(someVar"), "call should be preserved: {out}");
    }

    #[test]
    fn preserves_unknown_dispatcher() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0]; } break;
                }
            }
            unknown(X, [5]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 0, "should not inline unknown function: {out}");
        assert!(out.contains("unknown(X"), "unknown call should be preserved: {out}");
    }

    #[test]
    fn handles_call_this_form() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0] + 1; } break;
                }
            }
            D.call(this, X, [5]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline .call(this,...) form: {out}");
        assert!(
            out.contains("var a = [5]"),
            "should have var decl for args param: {out}"
        );
    }

    #[test]
    fn handles_multiple_cases() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case A: { return a[0] + 1; } break;
                    case B: { return a[0] * 2; } break;
                    case C: { return a[0] - 3; } break;
                }
            }
            D(A, [1]);
            D(B, [2]);
            D(C, [3]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 3, "should inline all 3 calls: {out}");
    }

    #[test]
    fn counts_inlines() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { foo(); } break;
                    case Y: { bar(); } break;
                }
            }
            D(X, [1]);
            D(Y, [2]);
            D(X, [3]);
        ";
        let (_, inlined) = run(code);
        assert_eq!(inlined, 3);
    }

    #[test]
    fn iife_contains_case_body() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { var t = a[0]; foo(t); } break;
                }
            }
            D(X, [42]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1);
        assert!(out.contains("foo(t)"), "case body should be in IIFE: {out}");
        assert!(out.contains("var t = a[0]"), "var decl should be in IIFE: {out}");
    }

    #[test]
    fn skips_wrong_arg_count() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { return a[0]; } break;
                }
            }
            D(X);
            D(X, [1], extra);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 0, "wrong arg count should not inline: {out}");
    }

    #[test]
    fn handles_nested_in_iife() {
        let code = r"
            (function() {
                function D(s, a) {
                    switch (s) {
                        case X: { return a[0]; } break;
                    }
                }
                D(X, [1]);
            })();
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline inside IIFE wrapper: {out}");
    }

    #[test]
    fn no_args_ref_emits_bare_stmts() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { foo(); bar(); } break;
                }
            }
            D(X, [1]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline: {out}");
        assert!(!out.contains("function("), "no IIFE wrapper: {out}");
        assert!(!out.contains("var a"), "no var decl for unused param: {out}");
        assert!(out.contains("foo()"), "bare stmt emitted: {out}");
        assert!(out.contains("bar()"), "bare stmt emitted: {out}");
    }

    #[test]
    fn args_ref_emits_block_with_var() {
        let code = r"
            function D(s, a) {
                switch (s) {
                    case X: { var t = a[0]; foo(t); } break;
                }
            }
            D(X, [42]);
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "should inline: {out}");
        assert!(!out.contains("function(a)"), "no IIFE param: {out}");
        assert!(out.contains("var a = [42]"), "var decl for args param: {out}");
        assert!(out.contains("foo(t)"), "body preserved: {out}");
    }

    #[test]
    fn call_this_form_keeps_this_in_an_arrow_iife() {
        let code = r"
            function D(s, a) { switch (s) { case X: { return this.v + a[0]; } } }
            var o = { v: 10, m: function () { var r = D.call(this, X, [5]); return r; } };
        ";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "{out}");
        assert!(out.contains("var r = (() => {"), "expected an arrow IIFE: {out}");
    }

    #[test]
    fn skips_bodies_reading_the_rebound_context() {
        for body in [
            "out.push(this)",
            "out.push(arguments.length)",
            "out.push(new.target)",
            "eval(c)",
        ] {
            let code = format!(
                "function D(s, a) {{ switch (s) {{ case X: {{ {body}; }} break; }} }}\nvar o = {{ m: function () {{ D(X, [5]); }} }};"
            );
            let (out, inlined) = run(&code);
            assert_eq!(inlined, 0, "{body}: {out}");
        }
        let code =
            "function D(s, a) { switch (s) { case X: { out.push(arguments.length); } break; } }\nD.call(this, X, [5]);";
        assert_eq!(run(code).1, 0);
    }

    #[test]
    fn return_does_not_leave_the_caller() {
        let code = r#"
            function D(s, a) { switch (s) { case X: { return 1; } } }
            function f() { D(X, [5]); return "reached"; }
        "#;
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "{out}");
        assert!(out.contains("(() => {"), "a returning body must stay in an IIFE: {out}");
    }

    #[test]
    fn skips_bodies_whose_names_resolve_differently_at_the_call_site() {
        let state_param = "function D(s, a) { switch (s) { case X: { foo(s); } break; } }\nD(X, [1]);";
        assert_eq!(run(state_param).1, 0);
        let shadowed =
            "var t = 1;\nfunction D(s, a) { switch (s) { case X: { foo(t); } break; } }\nfunction f(t) { D(X, [1]); }";
        assert_eq!(run(shadowed).1, 0);
        let label = "function D(s, a) { switch (s) { case X: { foo(); } break; } }\nfunction f(X) { D(X, [1]); }";
        assert_eq!(run(label).1, 0);
        let callee = "function D(s, a) { switch (s) { case X: { foo(); } break; } }\nfunction f(D) { D(X, [1]); }";
        assert_eq!(run(callee).1, 0);
    }

    #[test]
    fn skips_cases_that_fall_through_or_jump_out() {
        let fallthrough = "function D(s, a) { switch (s) { case X: foo(); case Y: bar(); break; } }\nD(X, [1]);";
        assert_eq!(run(fallthrough).1, 0);
        let nested_break = "function D(s, a) { switch (s) { case X: if (a[0]) break; foo(); break; } }\nD(X, [1]);";
        assert_eq!(run(nested_break).1, 0);
        let after_switch = "function D(s, a) { switch (s) { case X: foo(); break; } bar(); }\nD(X, [1]);";
        assert_eq!(run(after_switch).1, 0);
        let inner_loop = "function D(s, a) { switch (s) { case X: for (;;) { if (a[0]) break; } break; } }\nD(X, [1]);";
        assert_eq!(run(inner_loop).1, 1);
    }

    #[test]
    fn keeps_args_evaluation_when_the_body_ignores_them() {
        let code = "function D(s, a) { switch (s) { case X: { foo(); } break; } }\nD(X, [g()]);";
        let (out, inlined) = run(code);
        assert_eq!(inlined, 1, "{out}");
        assert!(out.contains("var a = [g()]"), "args must still be evaluated: {out}");
    }
}
