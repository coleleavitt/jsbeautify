//! Webpack 4/5 bundle naming.
//!
//! Minified webpack output hides its module system behind single-letter
//! bindings. This pass recovers the names webpack itself uses in development
//! builds, renaming by resolved [`SymbolId`] only:
//!
//! - the bootstrap require function, its module-id parameter, the module map
//!   and the module cache become `__webpack_require__`, `moduleId`,
//!   `__webpack_modules__` and `__webpack_module_cache__`;
//! - module factory parameters (in the bootstrap module map, the webpack 4
//!   bootstrap IIFE argument, and JSONP / `importScripts` chunk payloads) become
//!   `module`, `exports`, `__webpack_require__`, in that positional order;
//! - locals exposed through `__webpack_require__.d(exports, …)` take their
//!   public export name;
//! - `var x = __webpack_require__(123)` becomes `module_123`, and its
//!   `__webpack_require__.n(x)` interop getter `module_123_default`.
//!
//! Runtime property names (`.d`, `.r`, `.n`, `.o`, …) are left untouched: other
//! chunks reach them by name. Names that would collide with, shadow, or be
//! captured by another binding get a `$N` suffix (see
//! [`crate::ast_deobfuscate::rename_planner`]); bindings
//! visible to a direct `eval` or a `with` body, and native ES exports, are never
//! renamed.

mod collector;
mod runtime;
pub(super) mod shapes;

#[cfg(test)]
mod tests;

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::SemanticBuilder;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

use crate::ast_deobfuscate::rename_planner::apply_rename_plan;
use crate::ast_deobfuscate::state::DeobfuscateState;

use collector::NameCollector;
use runtime::RuntimeCollector;

/// Rename webpack runtime, factory, export and import bindings in `program`.
/// Inert on non-webpack input.
pub fn rename_webpack_bindings<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
    eprintln!("[DEOBFUSCATE] Phase 0.85: webpack check");
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);

    let mut runtime = RuntimeCollector::default();
    traverse_mut_with_ctx(&mut runtime, program, &mut ctx);
    let has_chunk = runtime.has_chunk();
    let runtimes = runtime.into_runtimes();
    let kind = match (runtimes.is_empty(), has_chunk) {
        (true, false) => {
            eprintln!("[DEOBFUSCATE] Phase 0.85: webpack check: not found");
            return;
        }
        (false, false) => "runtime",
        (true, true) => "chunk",
        (false, true) => "runtime and chunk",
    };

    let mut names = NameCollector::new(&runtimes);
    traverse_mut_with_ctx(&mut names, program, &mut ctx);
    eprintln!(
        "[DEOBFUSCATE] Phase 0.85: webpack check: detected webpack {kind} with {} modules",
        names.module_count()
    );
    let (plan, opaque_scopes) = names.into_plan();
    if plan.is_empty() {
        return;
    }

    let counts = apply_rename_plan(allocator, program, ctx, plan, &opaque_scopes);
    eprintln!(
        "[DEOBFUSCATE] Phase 0.85: Renamed {} webpack bindings ({} declarations, {} references)",
        counts.planned, counts.declarations, counts.references
    );
}

/// One detected webpack bootstrap.
struct Runtime {
    require: SymbolId,
    module_id: SymbolId,
    modules: SymbolId,
    cache: Option<SymbolId>,
}
