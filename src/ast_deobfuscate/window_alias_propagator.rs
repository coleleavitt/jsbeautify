//! Window-alias propagator: `var Q6 = window; Q6.document` → `document`.
//!
//! Detects variables assigned only to global-like values (`window`, `globalThis`,
//! `global`, `self`, `this`) and replaces `alias.prop` with just `prop`.

use oxc_ast::ast::{AssignmentExpression, AssignmentTarget, Expression, IdentifierReference, VariableDeclarator};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashSet;
use std::cell::Cell;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

fn is_global_like(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::Identifier(id)
        if matches!(id.name.as_str(), "window" | "globalThis" | "global" | "self"))
        || matches!(expr, Expression::ThisExpression(_))
}

pub struct WindowAliasCollector {
    candidates: FxHashSet<String>,
    poisoned: FxHashSet<String>,
}

impl WindowAliasCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: FxHashSet::default(),
            poisoned: FxHashSet::default(),
        }
    }

    #[must_use]
    pub fn aliases(self) -> FxHashSet<String> {
        &self.candidates - &self.poisoned
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasCollector {
    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let Some(name) = decl.id.get_identifier_name() else {
            return;
        };
        let Some(init) = &decl.init else {
            return;
        };
        // `var X = window` makes X a candidate; `var X = other` in an inner scope
        // creates a new binding and does NOT reassign the outer X, so we only
        // record candidates here, never poison.
        if is_global_like(init) {
            self.candidates.insert(name.to_string());
        }
    }

    fn enter_assignment_expression(&mut self, assign: &mut AssignmentExpression<'a>, _ctx: &mut Ctx<'a>) {
        let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left else {
            return;
        };
        let name = id.name.as_str();
        if is_global_like(&assign.right) {
            self.candidates.insert(name.to_string());
        } else {
            self.poisoned.insert(name.to_string());
        }
    }
}

pub struct WindowAliasPropagator {
    aliases: FxHashSet<String>,
    rewrites: usize,
}

impl WindowAliasPropagator {
    #[must_use]
    pub fn new(aliases: FxHashSet<String>) -> Self {
        Self { aliases, rewrites: 0 }
    }

    #[must_use]
    pub fn rewrites(&self) -> usize {
        self.rewrites
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for WindowAliasPropagator {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::StaticMemberExpression(sme) = expr else {
            return;
        };
        let Expression::Identifier(obj) = &sme.object else {
            return;
        };
        if !self.aliases.contains(obj.name.as_str()) {
            return;
        }
        let prop_name = sme.property.name.as_str();
        self.rewrites += 1;
        *expr = Expression::Identifier(ctx.ast.alloc(IdentifierReference {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            name: ctx.ast.ident(prop_name),
            reference_id: Cell::default(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;

        let mut collector = WindowAliasCollector::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut collector, &mut program, &mut ctx);

        let aliases = collector.aliases();
        if aliases.is_empty() {
            return Codegen::new().build(&program).code;
        }

        let mut propagator = WindowAliasPropagator::new(aliases);
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut propagator, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn propagates_window_alias() {
        let out = run("var Q6 = window; var x = Q6.document;");
        assert!(
            out.contains("var x = document"),
            "expected Q6.document → document, got: {out}"
        );
        assert!(!out.contains("Q6.document"), "Q6.document should be gone, got: {out}");
    }

    #[test]
    fn preserves_reassigned_alias() {
        let out = run("var Q6 = window; Q6 = other; var x = Q6.document;");
        assert!(
            out.contains("Q6.document"),
            "reassigned alias should not be propagated, got: {out}"
        );
    }

    #[test]
    fn preserves_computed_access() {
        let out = run("var Q6 = window; var x = Q6[\"doc\"];");
        assert!(
            out.contains("Q6[") || out.contains("Q6."),
            "computed access should not be rewritten, got: {out}"
        );
    }
}
