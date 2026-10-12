use super::*;
use crate::ast_deobfuscate::dowhile_switch_detector::{
    DoWhileSwitchDetector, collect_constants, resolve_compound_transitions,
};
use crate::ast_deobfuscate::semantic;
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

fn run(code: &str) -> (String, usize) {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;

    let detector = DoWhileSwitchDetector::new();
    let dispatchers = detector.detect(&program);
    if dispatchers.is_empty() {
        return (Codegen::new().build(&program).code, 0);
    }

    let mut cleaner = DoWhileSwitchCleaner::new(dispatchers, &program);

    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut cleaner, &mut program, &mut ctx);

    let pruned = cleaner.pruned_cases();
    (Codegen::new().build(&program).code, pruned)
}

#[test]
fn prunes_unreachable_case() {
    // Dispatcher with 3 cases: A→B (sequential), B→EXIT (loop exit), C→EXIT (loop exit)
    // Only entry point is A, so A→B are reachable, C is dead.
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { foo(); s = B; } break;
                        case B: { bar(); s = EXIT; } break;
                        case C: { baz(); s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 1, "should prune 1 dead case (C): {out}");
    assert!(out.contains("case A:"), "case A should be kept: {out}");
    assert!(out.contains("case B:"), "case B should be kept: {out}");
    assert!(!out.contains("case C:"), "case C should be pruned: {out}");
}

#[test]
fn preserves_all_reachable_cases() {
    // All cases reachable: entry=A, A→B, B→C, C→EXIT
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { foo(); s = B; } break;
                        case B: { bar(); s = C; } break;
                        case C: { baz(); s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 0, "should prune nothing: {out}");
    assert!(out.contains("case A:"), "case A should be kept: {out}");
    assert!(out.contains("case B:"), "case B should be kept: {out}");
    assert!(out.contains("case C:"), "case C should be kept: {out}");
}

#[test]
fn handles_transitive_reachability() {
    // Entry=A, A→B, B→C, C→EXIT. D is dead.
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = B; } break;
                        case B: { s = C; } break;
                        case C: { s = EXIT; } break;
                        case D: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 1, "should prune 1 dead case (D): {out}");
    assert!(out.contains("case A:"), "A reachable: {out}");
    assert!(out.contains("case B:"), "B reachable via A: {out}");
    assert!(out.contains("case C:"), "C reachable via A→B: {out}");
    assert!(!out.contains("case D:"), "D unreachable: {out}");
}

#[test]
fn counts_pruned() {
    // Entry=A, A→EXIT. B, C, D are all dead.
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = EXIT; } break;
                        case B: { s = EXIT; } break;
                        case C: { s = EXIT; } break;
                        case D: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (_, pruned) = run(code);
    assert_eq!(pruned, 3, "should prune 3 dead cases (B, C, D)");
}

#[test]
fn prunes_with_conditional_edges() {
    // A→B (sequential), B→C or D (conditional), entry=A → A,B,C,D reachable. E is dead.
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = B; } break;
                        case B: { if (a[0]) { s = C; } else { s = D; } } break;
                        case C: { s = EXIT; } break;
                        case D: { s = EXIT; } break;
                        case E: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 1, "should prune 1 dead case (E): {out}");
    assert!(out.contains("case A:"), "A reachable: {out}");
    assert!(out.contains("case B:"), "B reachable: {out}");
    assert!(out.contains("case C:"), "C reachable via conditional: {out}");
    assert!(out.contains("case D:"), "D reachable via conditional: {out}");
    assert!(!out.contains("case E:"), "E unreachable: {out}");
}

#[test]
fn unknown_preserves_all() {
    // A has Unknown transition (complex body), B and C exist. Entry=A.
    // Unknown conservatively reaches all → nothing pruned.
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { foo(); } break;
                        case B: { s = EXIT; } break;
                        case C: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 0, "Unknown transition should preserve all: {out}");
    assert!(out.contains("case A:"), "A kept: {out}");
    assert!(out.contains("case B:"), "B kept (reachable via Unknown): {out}");
    assert!(out.contains("case C:"), "C kept (reachable via Unknown): {out}");
}

#[test]
fn bfs_follows_chain() {
    // A→B→C→D→EXIT, entry=A, E is dead.
    let code = r"
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case A: { s = B; } break;
                        case B: { s = C; } break;
                        case C: { s = D; } break;
                        case D: { s = EXIT; } break;
                        case E: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(A, []);
        ";
    let (out, pruned) = run(code);
    assert_eq!(pruned, 1, "should prune 1 dead case (E): {out}");
    assert!(out.contains("case A:"), "A reachable: {out}");
    assert!(out.contains("case B:"), "B reachable: {out}");
    assert!(out.contains("case C:"), "C reachable: {out}");
    assert!(out.contains("case D:"), "D reachable: {out}");
    assert!(!out.contains("case E:"), "E unreachable: {out}");
}

fn run_with_compounds(code: &str) -> (String, usize) {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
    let mut program = ret.program;

    let detector = DoWhileSwitchDetector::new();
    let mut dispatchers = detector.detect(&program);
    if dispatchers.is_empty() {
        return (Codegen::new().build(&program).code, 0);
    }

    let constants = collect_constants(&program);
    for info in dispatchers.values_mut() {
        resolve_compound_transitions(info, &constants);
    }

    let mut cleaner = DoWhileSwitchCleaner::new(dispatchers, &program);
    let scoping = semantic::scoping(&program);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
    traverse_mut_with_ctx(&mut cleaner, &mut program, &mut ctx);

    let pruned = cleaner.pruned_cases();
    (Codegen::new().build(&program).code, pruned)
}

#[test]
fn pruning_with_resolved_compounds() {
    let code = r"
            var X = 10;
            var C = 3;
            var Y = 13;
            var EXIT = 99;
            var DEAD1 = 50;
            var DEAD2 = 60;
            var F = function G(s, a) {
                do {
                    switch (s) {
                        case X: { foo(); s += C; } break;
                        case Y: { bar(); s = EXIT; } break;
                        case DEAD1: { s = EXIT; } break;
                        case DEAD2: { s = EXIT; } break;
                    }
                } while (s != EXIT);
            };
            F(X, []);
        ";
    let (out, pruned) = run_with_compounds(code);
    assert_eq!(pruned, 2, "should prune 2 dead cases (DEAD1, DEAD2): {out}");
    assert!(out.contains("case X:"), "X reachable (entry): {out}");
    assert!(out.contains("case Y:"), "Y reachable via compound X+=C: {out}");
    assert!(!out.contains("case DEAD1:"), "DEAD1 unreachable: {out}");
    assert!(!out.contains("case DEAD2:"), "DEAD2 unreachable: {out}");
}
