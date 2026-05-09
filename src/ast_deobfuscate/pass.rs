//! Trait-based pass infrastructure for the deobfuscation pipeline.
//!
//! Inspired by rustc's MIR pass manager: passes declare their phase,
//! the pipeline groups them by phase and runs them in order. Each pass
//! reports whether it made changes, enabling fixed-point iteration.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::SemanticBuilder;
use oxc_traverse::{ReusableTraverseCtx, Traverse, traverse_mut_with_ctx};

use super::state::DeobfuscateState;

/// Which phase of the pipeline a pass belongs to.
/// Passes within the same phase may share semantic context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Early: detection and collection (Akamai, Bun, esbuild, window aliases)
    Detection = 0,
    /// Mid: structural transforms (control flow, string arrays, dispatchers, inlining)
    Structural = 1,
    /// Late: simplification and cleanup (constant folding, dead code, expression simplify)
    Simplification = 2,
    /// Final: renaming, annotation, and formatting
    Cosmetic = 3,
}

/// Result of running a single pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassResult {
    /// Pass made at least one modification to the AST.
    Changed,
    /// Pass ran but made no modifications.
    Unchanged,
    /// Pass was skipped (e.g. not applicable to this input).
    Skipped { reason: &'static str },
}

impl PassResult {
    #[must_use]
    pub fn is_changed(&self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// Core trait for deobfuscation passes.
///
/// Each pass declares its name, phase, and implements `run()` which receives
/// the mutable AST program and the allocator. The pass is responsible for
/// building any semantic context it needs internally.
pub trait DeobfuscatePass: Send + Sync {
    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &'static str;

    /// Which pipeline phase this pass belongs to.
    fn phase(&self) -> Phase;

    /// Execute the pass on the program. Returns whether changes were made.
    fn run<'a>(&mut self, program: &mut Program<'a>, alloc: &'a Allocator) -> PassResult;
}

/// Helper to run a single `Traverse`-based pass with fresh semantic context.
/// Avoids repeating the SemanticBuilder + ReusableTraverseCtx boilerplate.
pub fn run_traverse_pass<'a, T>(pass: &mut T, program: &mut Program<'a>, alloc: &'a Allocator)
where
    T: Traverse<'a, DeobfuscateState>,
{
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, alloc);
    traverse_mut_with_ctx(pass, program, &mut ctx);
}

/// Helper to run a `Traverse`-based pass with an existing `DeobfuscateState`.
/// Used when passes need to share state (e.g., string array info).
pub fn run_traverse_pass_with_state<'a, T>(
    pass: &mut T,
    program: &mut Program<'a>,
    alloc: &'a Allocator,
    state: DeobfuscateState,
) -> DeobfuscateState
where
    T: Traverse<'a, DeobfuscateState>,
{
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(state, scoping, alloc);
    traverse_mut_with_ctx(pass, program, &mut ctx);
    ctx.into_state()
}
