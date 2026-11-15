//! Akamai Bot Manager (BMP) specific deobfuscation passes.
//!
//! This module targets patterns produced by Akamai's Bot Manager JavaScript
//! (the `_abck` / `bm_sz` / `bmak` / `sensor_data` bundle). These patterns are
//! independent of the generic obfuscator.io / javascript-obfuscator patterns
//! handled elsewhere in `ast_deobfuscate`, and are gated behind a signature
//! detector so non-Akamai inputs are untouched.
//!
//! # Detection
//!
//! [`AkamaiDetector`] scans a program for BMP fingerprints:
//! - `bmak`, `_abck`, `bm_sz`, `ak_bmsc`, `bm_sv`, `sensor_data` as identifiers or string literals
//! - self-initialising accessor functions (`function F() { var x = ...; F = function(){return x}; return x; }`)
//! - a top-level `var Ot;` stack-tracker that is only written to
//!
//! A minimum number of distinct signatures must be present before the passes fire.
//!
//! # Passes
//!
//! 1. [`BooleanArithmeticFolder`] — collapses `+true + true + true + true + true`
//!    into `5`, handles `true + true`, `[+true] + [0] - +true`, etc.
//! 2. [`UndefinedPatternNormalizer`] — replaces `[][[]]` with the identifier
//!    `undefined` (BMP uses this as a short undefined literal).
//! 3. [`EqualityProxyUnwrapper`] — detects 2-argument proxy functions of the
//!    shape `function BI(a,b){return a===b;}` / `a!==b` / `a==b` / `a!=b` and
//!    inlines their call sites as binary expressions.
//! 4. [`StackTrackerRemover`] — finds a top-level variable that is only used
//!    via `.push(...)`, `.pop()`, and `.splice(...)` and deletes every such
//!    call site (and the declaration). This eliminates the pervasive BMP
//!    `Ot.push(Zl); ... Ot.pop();` instrumentation.
//! 5. [`SelfInitAccessorAnnotator`] — annotates (but does not rewrite) the
//!    self-initialising accessor pattern so downstream readers know `hx()`,
//!    `D8()`, `Fw()`, etc. are lookup-array wrappers, not real calls.
//!
//! All passes are safe to re-run and skip nodes they do not recognise.

use oxc_ast::ast::Program;
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::TraverseCtx;
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Whether every access to `symbol` at runtime is one of its resolved
/// references: it is declared once, is not a classic-script global that other
/// scripts reach by name, and it is not in `redirectable`
/// ([`crate::ast_deobfuscate::with_scope::redirectable_symbols`]: a `with`
/// object, an Annex B block function or a direct `eval` may stand in for it).
fn only_local_access(
    program: &Program<'_>,
    scoping: &Scoping,
    redirectable: &FxHashSet<SymbolId>,
    symbol: SymbolId,
) -> bool {
    scoping.symbol_redeclarations(symbol).is_empty()
        && !redirectable.contains(&symbol)
        && !is_global_binding(program.source_type, scoping, symbol)
}

mod boolean_arithmetic;
mod detector;
mod equality_proxy;
mod orchestrator;
mod stack_tracker;
mod undefined_pattern;

pub use boolean_arithmetic::BooleanArithmeticFolder;
pub use detector::AkamaiDetector;
pub use equality_proxy::EqualityProxyUnwrapper;
pub use orchestrator::AkamaiDeobfuscator;
pub use stack_tracker::StackTrackerRemover;
pub use undefined_pattern::UndefinedPatternNormalizer;

#[cfg(test)]
mod tests {
    use super::detector::walk_detector;
    use super::*;
    use crate::ast_deobfuscate::semantic;
    use oxc_allocator::Allocator;
    use oxc_ast::ast::BinaryOperator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    #[test]
    fn detects_bmp_by_identifier() {
        let allocator = Allocator::default();
        let code = "var bmak = {}; var _abck = '';";
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut deob = AkamaiDeobfuscator::new();
        assert!(deob.detect(&ret.program));
    }

    #[test]
    fn not_bmp_for_generic_code() {
        let allocator = Allocator::default();
        let code = "function add(a, b) { return a + b; } console.log(add(1, 2));";
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut deob = AkamaiDeobfuscator::new();
        assert!(!deob.detect(&ret.program));
    }

    #[test]
    fn folds_plus_true() {
        let allocator = Allocator::default();
        let code = "var x = +true + true + true + true + true;";
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut folder = BooleanArithmeticFolder::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut folder, &mut program, &mut ctx);
        let out = Codegen::new().build(&program).code;
        assert!(
            out.contains('5') || out.contains('4') || out.contains('3') || out.contains('2'),
            "expected partial fold of boolean arithmetic, got: {out}"
        );
    }

    #[test]
    fn normalizes_empty_array_brackets() {
        let allocator = Allocator::default();
        let code = "var x = [][[]];";
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut pass = UndefinedPatternNormalizer::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        assert_eq!(pass.replaced_count(), 1);
        let out = Codegen::new().build(&program).code;
        assert!(out.contains("undefined"), "expected undefined, got: {out}");
    }

    /// Runs the equality-proxy unwrapper; returns the output and the number
    /// of unwrapped calls.
    fn unwrap_eq(code: &str, source_type: SourceType) -> (String, usize) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(ret.diagnostics.is_empty(), "{code}: {:?}", ret.diagnostics);
        let mut program = ret.program;
        let scoping = semantic::scoping(&program);
        let mut unwrapper = EqualityProxyUnwrapper::new();
        unwrapper.collect(&program, &scoping);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut unwrapper, &mut program, &mut ctx);
        (Codegen::new().build(&program).code, unwrapper.unwrapped_count())
    }

    #[test]
    fn unwraps_equality_proxy() {
        let code = r#"
            var BI = function(a, b) { return a === b; };
            function c8(a, b) { return a < b; }
            var x = BI(typeof y, "undefined"), z = c8(1, 2);
        "#;
        let (out, unwrapped) = unwrap_eq(code, SourceType::mjs());
        assert!(out.contains("typeof y === \"undefined\""), "got: {out}");
        assert!(out.contains("z = 1 < 2"), "got: {out}");
        assert_eq!(unwrapped, 2);
    }

    #[test]
    fn equality_proxy_leaves_shadowing_bindings_alone() {
        let code = r"(function () {
            var BI = function (a, b) { return a === b; };
            function g(BI) { return BI(1, 2); }
            function h() { var BI = function (a, b) { return a !== b; }; return BI(1, 1); }
            function k(x) { { let BI = function (a, b) { return a < b; }; return BI(x, 2); } }
            return [g, h, k, BI(3, 3)];
        })();";
        let (out, unwrapped) = unwrap_eq(code, SourceType::cjs());
        assert_eq!(unwrapped, 1, "{out}");
        assert!(out.contains("3 === 3"), "{out}");
        assert_eq!(
            out.matches("BI(1, 2)").count() + out.matches("BI(1, 1)").count(),
            2,
            "{out}"
        );
        assert!(out.contains("BI(x, 2)"), "{out}");
    }

    #[test]
    fn equality_proxy_skips_bindings_reachable_another_way() {
        for code in [
            // Reassigned.
            "(function () { var BI = function (a, b) { return a === b; }; BI = function (a, b) { return a < b; }; return BI(1, 2); })();",
            // Redeclared.
            "(function () { var BI = function (a, b) { return a === b; }; var BI = function (a, b) { return a + b; }; return BI(1, 2); })();",
            // Duplicate parameters: both operands read the second argument.
            "(function () { var BI = function (a, a) { return a === a; }; return BI(1, 2); })();",
            // Operands swapped relative to the parameters.
            "(function () { var BI = function (a, b) { return b < a; }; return BI(1, 2); })();",
            // A direct eval may assign it, in its own scope or a nested one.
            "(function () { var BI = function (a, b) { return a === b; }; eval('BI = 0'); return BI(1, 2); })();",
            "(function () { var BI = function (a, b) { return a === b; }; function z() { eval('BI = 0'); } return BI(1, 2); })();",
            // A `with` object may hold a property of the same name.
            "(function () { var BI = function (a, b) { return a === b; }; with (o) { BI(1, 2); } })();",
            // An Annex B block function of the same name overwrites it.
            "(function () { function BI(a, b) { return a === b; } { function BI(a, b) { return a + b; } } return BI(1, 2); })();",
        ] {
            let (out, unwrapped) = unwrap_eq(code, SourceType::cjs());
            assert_eq!(unwrapped, 0, "{code}\n{out}");
        }
    }

    #[test]
    fn equality_proxy_skips_classic_script_globals() {
        let code = "var BI = function (a, b) { return a === b; }; BI(1, 2);";
        assert_eq!(unwrap_eq(code, SourceType::script()).1, 0);
        assert_eq!(unwrap_eq(code, SourceType::mjs()).1, 1);
    }

    /// Runs the stack-tracker remover; returns the output and the trackers.
    fn strip_tracker(code: &str, source_type: SourceType) -> (String, Vec<String>) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(ret.diagnostics.is_empty(), "{code}: {:?}", ret.diagnostics);
        let mut program = ret.program;
        let scoping = semantic::scoping(&program);
        let mut remover = StackTrackerRemover::new();
        remover.detect(&program, &scoping);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut remover, &mut program, &mut ctx);
        (Codegen::new().build(&program).code, remover.tracker_names().to_vec())
    }

    #[test]
    fn removes_stack_tracker_calls() {
        let code = r"
            var Ot;
            Ot = [];
            Ot.push(1);
            Ot.push(2), Ot.push(3);
            foo();
            Ot.pop();
            function f(Zl) { Ot.push(Zl); var r = Zl * 2; return Ot.pop(), r; }
            Ot.splice(0, 1, -9);
        ";
        let (out, trackers) = strip_tracker(code, SourceType::mjs());
        assert_eq!(trackers, ["Ot"]);
        assert!(!out.contains("Ot"), "got: {out}");
        assert!(out.contains("foo()") && out.contains("return r"), "got: {out}");
    }

    #[test]
    fn stack_tracker_keeps_bindings_with_other_uses() {
        let ops = "Ot = []; Ot.push(1); Ot.push(2); Ot.pop(); Ot.pop();";
        // A parameter of the same name is another array.
        let (out, trackers) = strip_tracker(
            &format!("var Ot; {ops} function add(Ot, v) {{ Ot.push(v); }}"),
            SourceType::mjs(),
        );
        assert_eq!(trackers, ["Ot"]);
        assert!(out.contains("Ot.push(v)") && !out.contains("Ot.pop"), "got: {out}");
        for code in [
            // The popped value is used.
            format!("var Ot; {ops} var x = (0, Ot.pop());"),
            // Assigned something other than a fresh array.
            format!("var Ot; {ops} Ot = shared;"),
            // A pushed argument has side effects or may throw.
            format!("var Ot; {ops} Ot.push(tick());"),
            format!("var Ot; {ops} Ot.push(undeclared);"),
            format!("var Ot; {ops} Ot.push(later); let later = 1;"),
            // Never reset to an array.
            "var Ot; Ot.push(1); Ot.push(2); Ot.pop(); Ot.pop();".to_string(),
            // Redeclared, exported, or visible to a direct eval or a `with` body.
            format!("var Ot; var Ot; {ops}"),
            format!("export var Ot; {ops}"),
            format!("var Ot; {ops} eval('Ot');"),
            format!("var Ot; {ops} with (o) {{ Ot.pop(); }}"),
        ] {
            let (out, trackers) = strip_tracker(&code, SourceType::mjs());
            assert!(trackers.is_empty(), "{code}\n{out}");
        }
        let (_, trackers) = strip_tracker(&format!("var Ot; {ops}"), SourceType::script());
        assert!(trackers.is_empty(), "a classic script's top-level var is a global");
    }

    #[test]
    fn detects_self_init_accessor() {
        let code = r"
            function hx() {
                var x = new Object();
                hx = function() { return x; };
                return x;
            }
        ";
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut deob = AkamaiDeobfuscator::new();
        deob.detect(&ret.program);
        // Not enough signatures on its own, but accessor count must be 1.
        let mut detector = AkamaiDetector::new();
        walk_detector(&ret.program.body, &mut detector);
        assert_eq!(detector.self_init_accessor_count(), 1);
    }
}
