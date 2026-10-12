//! Turbopack (Next.js 13.4+ / 16) chunk naming.
//!
//! Turbopack chunks register module factories with
//! `(globalThis.TURBOPACK || (globalThis.TURBOPACK = [])).push([script, id, factory, …])`,
//! or export them from a Node.js chunk with `module.exports = [id, factory, …]`
//! (see [`shapes`]). Every factory receives the module's `__turbopack_context__`
//! (minified to one letter), and `CommonJS` factories also `module` and `exports`.
//! The context's one-letter members are the runtime API (from
//! `turbopack-ecmascript-runtime`'s `runtime-utils.ts`): `s` esmExport
//! (`s(bindings, id?)`, where `id` names a module scope-hoisted into this
//! factory), `i` esmImport, `r` commonJsRequire, `A` asyncLoader, `a`
//! asyncModule, `v` exportValue, `n` exportNamespace, `j` dynamicExport,
//! `f` moduleContext, `t` runtimeRequire, `e` the exports object, `m` the module.
//!
//! This pass renames, by resolved [`SymbolId`] only:
//!
//! - factory parameters to `__turbopack_context__`, `module`, `exports`, and the
//!   async module body's parameters to `__turbopack_handle_async_dependencies__`,
//!   `__turbopack_async_result__` (Turbopack's own names before minification);
//! - locals exported through `__turbopack_context__.s([name, getter | 0, value, …])`
//!   to their export name, and a minified `default` export local to a name
//!   derived from its module id (see [`crate::ast_deobfuscate::default_export`]);
//! - `x = __turbopack_context__.i(123)` / `.r(123)` to `module_123`, and with a
//!   development id `.i("[project]/src/foo.ts [app-client] (ecmascript)")` to
//!   `module_src_foo_ts` (the id stays in the call); `x = __turbopack_context__.x("react", …)`
//!   to `external_react`;
//! - in the runtime chunk, the `Context` constructor, its `contextPrototype`
//!   alias and the functions installed on it (`contextPrototype.i = esmImport`),
//!   with Node.js names where the runtimes differ, and the Node.js runtime's
//!   `module.exports = (sourcePath) => ({ m, c })`;
//! - in the worker entrypoint (`turbopack-worker-<hash>.js`), `abort`, `params`,
//!   `param` and `chunkUrls`.
//!
//! Export keys that production builds mangle to one or two characters are not
//! used as names. Development chunk lists and Node.js entries are only
//! reported.
//!
//! Member names stay as they are: the runtime and other chunks reach them by
//! name. Collisions get a `$N` suffix and the shared rename guards apply (see
//! [`crate::ast_deobfuscate::rename_planner`]).

mod collector;
pub mod shapes;

#[cfg(test)]
mod tests;

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::SemanticBuilder;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

use crate::ast_deobfuscate::rename_planner::apply_rename_plan;
use crate::ast_deobfuscate::state::DeobfuscateState;

use collector::NameCollector;

/// Rename Turbopack factory, export, import and runtime bindings in `program`.
/// Inert on other input.
pub fn rename_turbopack_bindings<'a>(allocator: &'a Allocator, program: &mut Program<'a>) {
    eprintln!("[DEOBFUSCATE] Phase 0.86: Turbopack check");
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);

    let mut names = NameCollector::default();
    traverse_mut_with_ctx(&mut names, program, &mut ctx);
    if names.found.is_empty() && !names.has_runtime() {
        eprintln!("[DEOBFUSCATE] Phase 0.86: Turbopack check: not found");
        return;
    }
    let found = &names.found;
    let mut detected = Vec::new();
    if found.registrations > 0 {
        detected.push(format!(
            "{} registration(s), {} module factories",
            found.registrations, found.factories
        ));
    }
    if found.node_chunks > 0 {
        detected.push("Node.js chunk".to_string());
    }
    if found.chunk_lists > 0 {
        detected.push(format!("{} chunk list(s)", found.chunk_lists));
    }
    if found.node_entries > 0 {
        detected.push("Node.js entry".to_string());
    }
    if found.worker_entrypoint {
        detected.push("worker entrypoint".to_string());
    }
    if names.has_runtime() {
        detected.push("runtime".to_string());
    }
    eprintln!(
        "[DEOBFUSCATE] Phase 0.86: Turbopack check: detected {}",
        detected.join(", ")
    );
    let (plan, with_scopes) = names.into_plan();
    if plan.is_empty() {
        return;
    }
    let counts = apply_rename_plan(allocator, program, ctx, plan, &with_scopes);
    eprintln!(
        "[DEOBFUSCATE] Phase 0.86: Renamed {} Turbopack bindings ({} declarations, {} references)",
        counts.planned, counts.declarations, counts.references
    );
}
