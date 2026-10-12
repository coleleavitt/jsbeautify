use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::{Scoping, Semantic};
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::bun::export_renamer::make_names_unique;
use crate::ast_deobfuscate::class_name_renamer::ClassNameRenamer;
use crate::ast_deobfuscate::default_export::is_minified;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::semantic;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::collector::AngularNameCollector;

/// Rename minified Angular class bindings (from `ɵcmp`/`ɵdir`/`ɵpipe`
/// definitions) and `NgRx` action creators to derived names, by `SymbolId`.
///
/// Bindings visible to a direct `eval` or exported by name are skipped, names
/// are made unique along the scope chain, and a name that an unresolved
/// (global) reference already uses is dropped so no reference is captured.
/// Returns the number of rewritten identifiers. `keep_fn_names` as
/// [`RenameGuardCollector::keep_fn_names`].
///
/// `scoping` is a current analysis of `program`, if one is at hand; it is
/// returned when nothing was renamed.
pub fn rename_angular_symbols<'a>(
    allocator: &'a Allocator,
    program: &mut Program<'a>,
    scoping: Option<Scoping>,
    keep_fn_names: bool,
) -> (usize, Option<Scoping>) {
    eprintln!("[DEOBFUSCATE] Phase 0.995: Angular check");
    let scoping = semantic::reuse_or_build(program, scoping);
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = AngularNameCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    if collector.classes.is_empty() && collector.actions.is_empty() {
        eprintln!("[DEOBFUSCATE] Phase 0.995: Angular check: not found");
        return (0, Some(ctx.into_scoping()));
    }
    eprintln!(
        "[DEOBFUSCATE] Phase 0.995: Angular check: detected Angular with {} named classes, {} NgRx action creators",
        collector.classes.len(),
        collector.actions.len()
    );

    let mut guard = RenameGuardCollector::new().keep_fn_names(keep_fn_names);
    traverse_mut_with_ctx(&mut guard, program, &mut ctx);
    drop(ctx);

    let semantic = semantic::build_with(program, |builder| builder.with_build_nodes(true)).semantic;
    let guarded = guard.into_guarded(semantic.scoping());
    let plan = plan_renames(&collector, &semantic, &guarded);
    let scoping = semantic.into_scoping();
    if plan.is_empty() {
        return (0, Some(scoping));
    }

    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut renamer = ClassNameRenamer::new(plan);
    traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
    (renamer.renamed(), None)
}

fn plan_renames(
    collector: &AngularNameCollector,
    semantic: &Semantic<'_>,
    guarded: &FxHashSet<SymbolId>,
) -> FxHashMap<SymbolId, String> {
    let scoping = semantic.scoping();
    let renamable = |symbol: SymbolId| !guarded.contains(&symbol) && is_minified(scoping.symbol_name(symbol));

    let mut desired: FxHashMap<SymbolId, String> = FxHashMap::default();
    for (&class, name) in &collector.classes {
        let target = collector.wrappers.get(&class).map_or(class, |w| w.outer);
        desired.insert(target, name.clone());
    }
    for (&action, name) in &collector.actions {
        if !scoping.symbol_is_mutated(action) {
            desired.entry(action).or_insert_with(|| name.clone());
        }
    }
    desired.retain(|&symbol, _| renamable(symbol));

    let globals: FxHashSet<&str> = scoping
        .root_unresolved_references()
        .keys()
        .map(oxc_ast::ast::Ident::as_str)
        .collect();
    let mut plan = make_names_unique(desired, scoping, &FxHashSet::default());
    plan.retain(|_, name| !globals.contains(name.as_str()));

    // The class's own binding takes the variable's name unless the
    // initializer reads the variable, which the shared name would capture.
    for (&class, wrapper) in &collector.wrappers {
        if !collector.classes.contains_key(&class) || !renamable(class) {
            continue;
        }
        let Some(name) = plan.get(&wrapper.outer).cloned() else {
            continue;
        };
        let captured = scoping.get_resolved_references(wrapper.outer).any(|reference| {
            let span = semantic.nodes().get_node(reference.node_id()).span();
            wrapper.init_span.start <= span.start && span.end <= wrapper.init_span.end
        });
        if !captured {
            plan.insert(class, name);
        }
    }
    plan
}
