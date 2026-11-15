//! Applying a rename plan keyed by [`SymbolId`].
//!
//! [`SymbolRenamer`] gives each planned symbol its new name at every binding
//! and every reference that scoping resolves to it. Symbols that share a name
//! but not a [`SymbolId`] stay apart, so shadowing bindings keep their names.
//! Before a renamed identifier is reached, the shorthand properties that
//! spell it are written out (see [`crate::ast_deobfuscate::shorthand_targets`]).

use oxc_ast::ast::{AssignmentTargetProperty, BindingIdentifier, Ident, IdentifierReference, ObjectProperty};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::shorthand_targets::{expand_renamed_proto_shorthand, expand_renamed_shorthand};
use crate::ast_deobfuscate::state::DeobfuscateState;

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Renames each symbol of a plan at its declarations and its resolved
/// references, and counts the identifiers it changes.
pub struct SymbolRenamer {
    plan: FxHashMap<SymbolId, String>,
    renamed_decls: usize,
    renamed_refs: usize,
}

impl SymbolRenamer {
    #[must_use]
    pub const fn new(plan: FxHashMap<SymbolId, String>) -> Self {
        Self {
            plan,
            renamed_decls: 0,
            renamed_refs: 0,
        }
    }

    /// Binding identifiers renamed.
    #[must_use]
    pub const fn renamed_decls(&self) -> usize {
        self.renamed_decls
    }

    /// Identifier references renamed.
    #[must_use]
    pub const fn renamed_refs(&self) -> usize {
        self.renamed_refs
    }

    #[must_use]
    pub const fn total_renamed(&self) -> usize {
        self.renamed_decls + self.renamed_refs
    }

    /// The planned name of `symbol`, when it differs from `current`.
    fn new_name(&self, symbol: Option<SymbolId>, current: &str) -> Option<&str> {
        self.plan
            .get(&symbol?)
            .map(String::as_str)
            .filter(|name| *name != current)
    }

    fn new_reference_name(&self, ident: &IdentifierReference<'_>, ctx: &Ctx<'_>) -> Option<&str> {
        let symbol = ident
            .reference_id
            .get()
            .and_then(|reference| ctx.scoping().get_reference(reference).symbol_id());
        self.new_name(symbol, &ident.name)
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SymbolRenamer {
    fn enter_assignment_target_property(&mut self, prop: &mut AssignmentTargetProperty<'a>, ctx: &mut Ctx<'a>) {
        expand_renamed_shorthand(prop, ctx, |ident| self.new_reference_name(ident, ctx).is_some());
    }

    fn enter_object_property(&mut self, prop: &mut ObjectProperty<'a>, ctx: &mut Ctx<'a>) {
        expand_renamed_proto_shorthand(prop, ctx, |ident| self.new_reference_name(ident, ctx).is_some());
    }

    fn exit_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        if let Some(name) = self.new_name(ident.symbol_id.get(), &ident.name) {
            ident.name = Ident::from_str_in(name, ctx);
            self.renamed_decls += 1;
        }
    }

    fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(name) = self.new_reference_name(ident, ctx) {
            ident.name = Ident::from_str_in(name, ctx);
            self.renamed_refs += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    use super::SymbolRenamer;
    use crate::ast_deobfuscate::{DeobfuscateState, semantic};

    /// Renames every symbol whose name is a key of `names` and returns the
    /// program on one line, with the renamer's (declaration, reference) counts.
    fn rename(code: &str, names: &[(&str, &str)]) -> (String, usize, usize) {
        let allocator = Allocator::default();
        let mut program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
        let scoping = semantic::scoping(&program);
        let plan = scoping
            .symbol_ids()
            .filter_map(|symbol| {
                let name = scoping.symbol_name(symbol);
                names
                    .iter()
                    .find(|(old, _)| *old == name)
                    .map(|(_, new)| (symbol, (*new).to_string()))
            })
            .collect();
        let mut renamer = SymbolRenamer::new(plan);
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
        let code = Codegen::new().build(&program).code;
        let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
        (code, renamer.renamed_decls(), renamer.renamed_refs())
    }

    fn rename_g(code: &str) -> String {
        rename(code, &[("g", "v0")]).0
    }

    #[test]
    fn renames_bindings_and_resolved_references() {
        let (code, decls, refs) = rename("function g(a) { return g(a); } g(1);", &[("g", "v0"), ("a", "x")]);
        assert_eq!(code, "function v0(x) { return v0(x); } v0(1);");
        assert_eq!((decls, refs), (2, 3));
    }

    #[test]
    fn leaves_a_shadowing_binding_with_the_same_name() {
        let allocator = Allocator::default();
        let code = "var g = 1; function f(g) { return g; } g;";
        let mut program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
        let scoping = semantic::scoping(&program);
        let outer = scoping.get_root_binding("g".into()).unwrap();
        let mut renamer = SymbolRenamer::new(std::iter::once((outer, "v0".to_string())).collect());
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);
        let out = Codegen::new().build(&program).code;
        assert_eq!(out, "var v0 = 1;\nfunction f(g) {\n\treturn g;\n}\nv0;\n");
        assert_eq!(renamer.total_renamed(), 2);
    }

    #[test]
    fn leaves_an_unresolved_reference_with_a_planned_name() {
        assert_eq!(
            rename("function f(h) { return h; } h;", &[("h", "x")]).0,
            "function f(x) { return x; } h;"
        );
    }

    #[test]
    fn counts_only_identifiers_whose_name_changes() {
        let (code, decls, refs) = rename("var g = 1; g;", &[("g", "g")]);
        assert_eq!(code, "var g = 1; g;");
        assert_eq!((decls, refs), (0, 0));
    }

    #[test]
    fn shorthand_assignment_target_keeps_its_key() {
        assert_eq!(rename_g("var g; ({ g } = o);"), "var v0; ({g: v0} = o);");
        assert_eq!(rename_g("var g; ({ g = 1 } = o);"), "var v0; ({g: v0 = 1} = o);");
        assert_eq!(rename_g("var g; [{ g }] = a;"), "var v0; [{g: v0}] = a;");
        assert_eq!(rename_g("var g; for ({ g } of a);"), "var v0; for ({g: v0} of a);");
    }

    #[test]
    fn shorthand_properties_and_patterns_keep_their_keys() {
        assert_eq!(rename_g("var g; f({ g });"), "var v0; f({ g: v0 });");
        assert_eq!(rename_g("var { g = 1 } = o;"), "var { g: v0 = 1 } = o;");
        assert_eq!(rename_g("function f({ g }) {}"), "function f({ g: v0 }) {}");
        assert_eq!(rename_g("var g; export { g };"), "var v0; export { v0 as g };");
        assert_eq!(
            rename_g("import { g } from \"m\"; f(g);"),
            "import { g as v0 } from \"m\"; f(v0);"
        );
    }

    #[test]
    fn shorthand_proto_property_stays_an_own_property() {
        assert_eq!(
            rename("var __proto__; f({ __proto__ });", &[("__proto__", "v0")]).0,
            "var v0; f({ [\"__proto__\"]: v0 });"
        );
    }

    #[test]
    fn shorthands_of_other_bindings_are_untouched() {
        assert_eq!(
            rename_g("var g, h; ({ h } = o); f({ h, __proto__: h });"),
            "var v0, h; ({h} = o); f({ h, __proto__: h });"
        );
    }
}
