use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashSet;

use crate::ast_deobfuscate::semantic;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::{DeadVarCollector, DeadVarEliminator};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeadVarSweepResult {
    candidate_symbols: usize,
    removed_declarators: usize,
    rejected: bool,
}

impl DeadVarSweepResult {
    #[must_use]
    pub const fn candidate_symbols(self) -> usize {
        self.candidate_symbols
    }

    #[must_use]
    pub const fn removed_declarators(self) -> usize {
        self.removed_declarators
    }

    #[must_use]
    pub const fn was_rejected(self) -> bool {
        self.rejected
    }
}

/// Collect and transactionally eliminate dead variable declarations.
///
/// Candidate discovery uses the owning AST without allocating a replacement.
/// If there are candidates, validation occurs in a disposable arena. Only a
/// semantically validated candidate is printed and parsed into the owning arena.
/// A rejected candidate therefore leaves no candidate AST in the owning arena.
pub fn eliminate_dead_vars<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> DeadVarSweepResult {
    let (collector, _) = collect_semantic_state(allocator, program);
    let dead_vars = collector.get_dead_vars();
    let candidate_symbols = dead_vars.len();
    if dead_vars.is_empty() {
        return DeadVarSweepResult::default();
    }

    validate_and_commit(allocator, program, candidate_symbols, None)
}

#[cfg(test)]
pub(super) fn eliminate_dead_vars_with_forced_candidates<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    dead_vars: FxHashSet<SymbolId>,
) -> DeadVarSweepResult {
    let candidate_symbols = dead_vars.len();
    if dead_vars.is_empty() {
        return DeadVarSweepResult::default();
    }

    validate_and_commit(allocator, program, candidate_symbols, Some(dead_vars))
}

fn validate_and_commit<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    candidate_symbols: usize,
    forced_dead_vars: Option<FxHashSet<SymbolId>>,
) -> DeadVarSweepResult {
    let source_type = program.source_type;
    let original_source = Codegen::new().build(program).code;

    let candidate_allocator = Allocator::default();
    let parsed = Parser::new(&candidate_allocator, &original_source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return rejected(candidate_symbols);
    }
    let mut candidate = parsed.program;

    let (before_collector, before_error_count) = collect_semantic_state(&candidate_allocator, &mut candidate);
    if before_collector.resolution_failed() {
        return rejected(candidate_symbols);
    }
    let before_unresolved = before_collector.unresolved_references().clone();
    let dead_vars = forced_dead_vars.unwrap_or_else(|| before_collector.get_dead_vars());
    if dead_vars.is_empty() {
        return DeadVarSweepResult {
            candidate_symbols,
            removed_declarators: 0,
            rejected: false,
        };
    }

    let scoping = semantic::scoping(&candidate);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &candidate_allocator);
    let mut eliminator = DeadVarEliminator::new(dead_vars);
    traverse_mut_with_ctx(&mut eliminator, &mut candidate, &mut ctx);
    let removed_declarators = eliminator.removed_declarators();
    if removed_declarators == 0 {
        return DeadVarSweepResult {
            candidate_symbols,
            removed_declarators: 0,
            rejected: false,
        };
    }

    let (after_collector, after_error_count) = collect_semantic_state(&candidate_allocator, &mut candidate);
    let introduced_unresolved = !after_collector.unresolved_references().is_subset(&before_unresolved);
    let invalid_resolution = after_collector.resolution_failed();
    let introduced_errors = after_error_count > before_error_count;
    if introduced_unresolved || invalid_resolution || introduced_errors {
        return rejected(candidate_symbols);
    }

    let accepted_source = Codegen::new().build(&candidate).code;
    let verification_allocator = Allocator::default();
    let verified = Parser::new(&verification_allocator, &accepted_source, source_type).parse();
    let verified_semantic = semantic::build(&verified.program);
    if !verified.diagnostics.is_empty() || !verified_semantic.diagnostics.is_empty() {
        return rejected(candidate_symbols);
    }

    let owned_source = allocator.alloc_str(&accepted_source);
    let committed = Parser::new(allocator, owned_source, source_type).parse();
    debug_assert!(
        committed.diagnostics.is_empty(),
        "validated dead-var output failed deterministic reparse"
    );
    *program = committed.program;
    semantic::carry_stats_to(program);

    DeadVarSweepResult {
        candidate_symbols,
        removed_declarators,
        rejected: false,
    }
}

const fn rejected(candidate_symbols: usize) -> DeadVarSweepResult {
    DeadVarSweepResult {
        candidate_symbols,
        removed_declarators: 0,
        rejected: true,
    }
}

fn collect_semantic_state<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> (DeadVarCollector, usize) {
    let semantic = semantic::build(program);
    let error_count = semantic.diagnostics.len();
    let scoping = semantic.semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = DeadVarCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    (collector, error_count)
}
