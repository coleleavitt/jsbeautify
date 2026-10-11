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
//! captured by another binding get a `$N` suffix (see [`planner`]); bindings
//! visible to a direct `eval` or a `with` body, and native ES exports, are never
//! renamed.

mod collector;
mod planner;
mod runtime;
mod shapes;

#[cfg(test)]
mod tests;

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::SemanticBuilder;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

use crate::ast_deobfuscate::ExportNameRenamer;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::state::DeobfuscateState;

use collector::NameCollector;
use runtime::RuntimeCollector;

/// Rename webpack runtime, factory, export and import bindings in `program`.
/// Inert on non-webpack input.
pub fn rename_webpack_bindings<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
    eprintln!("[DEOBFUSCATE] Phase 0.85: Webpack module naming");
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);

    let mut runtime = RuntimeCollector::default();
    traverse_mut_with_ctx(&mut runtime, program, &mut ctx);
    let has_chunk = runtime.has_chunk();
    let runtimes = runtime.into_runtimes();
    if runtimes.is_empty() && !has_chunk {
        eprintln!("[DEOBFUSCATE] Phase 0.85: No webpack runtime or chunk found");
        return;
    }

    let mut names = NameCollector::new(&runtimes);
    traverse_mut_with_ctx(&mut names, program, &mut ctx);
    let (plan, opaque_scopes) = names.into_plan();
    if plan.is_empty() {
        return;
    }

    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, program, &mut ctx);
    let scoping = ctx.into_scoping();
    let mut guarded = guard.into_guarded(&scoping);
    guarded.extend(planner::symbols_visible_from(&opaque_scopes, &scoping));

    let names = planner::assign_names(plan, &guarded, &scoping);
    let planned = names.len();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut renamer = ExportNameRenamer::new(names);
    traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
    eprintln!(
        "[DEOBFUSCATE] Phase 0.85: Renamed {planned} webpack bindings ({} declarations, {} references)",
        renamer.renamed_decls(),
        renamer.renamed_refs()
    );
}

/// One detected webpack bootstrap.
struct Runtime {
    require: SymbolId,
    module_id: SymbolId,
    modules: SymbolId,
    cache: Option<SymbolId>,
}
