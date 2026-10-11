use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::GetSpan;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::bun::export_renamer::make_names_unique;
use crate::ast_deobfuscate::class_name_renamer::ClassNameRenamer;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::state::DeobfuscateState;

use super::collector::AngularNameCollector;
use super::naming::is_minified;

/// Rename minified Angular class bindings (from `ɵcmp`/`ɵdir`/`ɵpipe`
/// definitions) and NgRx action creators to derived names, by `SymbolId`.
///
/// Bindings visible to a direct `eval` or exported by name are skipped, names
/// are made unique along the scope chain, and a name that an unresolved
/// (global) reference already uses is dropped so no reference is captured.
/// Returns the number of rewritten identifiers.
pub fn rename_angular_symbols<'a>(allocator: &'a Allocator, program: &mut Program<'a>) -> usize {
    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut collector = AngularNameCollector::new();
    traverse_mut_with_ctx(&mut collector, program, &mut ctx);
    drop(ctx);
    if collector.classes.is_empty() && collector.actions.is_empty() {
        return 0;
    }

    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, program, &mut ctx);
    drop(ctx);

    let plan = {
        let semantic = SemanticBuilder::new().with_build_nodes(true).build(program).semantic;
        let guarded = guard.into_guarded(semantic.scoping());
        plan_renames(&collector, &semantic, &guarded)
    };
    if plan.is_empty() {
        return 0;
    }

    let scoping = SemanticBuilder::new().build(program).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, allocator);
    let mut renamer = ClassNameRenamer::new(plan);
    traverse_mut_with_ctx(&mut renamer, program, &mut ctx);
    renamer.renamed()
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
        .map(|k| k.as_str())
        .collect();
    let mut plan = make_names_unique(desired, scoping);
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
