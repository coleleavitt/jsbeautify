//! Shorthand destructuring targets that a rename would change.
//!
//! `({ x } = o)` and `({ x = d } = o)` spell the property key and the
//! assigned binding with one identifier. Renaming the binding in place also
//! renames the key, so the assignment would read another property. A renaming
//! pass calls [`expand_renamed_shorthand`] on every
//! [`AssignmentTargetProperty`] first, which spells the property out as
//! `({ x: x } = o)` when the binding is about to be renamed.

use oxc_allocator::{Box as ArenaBox, TakeIn};
use oxc_ast::ast::{
    AssignmentTarget, AssignmentTargetMaybeDefault, AssignmentTargetProperty, IdentifierReference, PropertyKey,
};
use oxc_traverse::TraverseCtx;

/// Turns `{ x }` / `{ x = d }` into `{ x: x }` / `{ x: x = d }` when
/// `renames(x)` says a pass will rename the binding `x`.
pub fn expand_renamed_shorthand<'a, State>(
    prop: &mut AssignmentTargetProperty<'a>,
    ctx: &TraverseCtx<'a, State>,
    renames: impl FnOnce(&IdentifierReference<'a>) -> bool,
) {
    let AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) = prop else {
        return;
    };
    if !renames(&shorthand.binding) {
        return;
    }
    let span = shorthand.span;
    let key = PropertyKey::new_static_identifier(shorthand.binding.span, shorthand.binding.name, ctx);
    let binding = shorthand.binding.take_in(ctx);
    let target = AssignmentTarget::AssignmentTargetIdentifier(ArenaBox::new_in(binding, &ctx.ast));
    let binding = match shorthand.init.take() {
        Some(init) => AssignmentTargetMaybeDefault::new_assignment_target_with_default(span, target, init, ctx),
        None => AssignmentTargetMaybeDefault::from(target),
    };
    *prop = AssignmentTargetProperty::new_assignment_target_property_property(span, key, binding, false, ctx);
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{AssignmentTargetProperty, Ident, IdentifierReference};
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};

    use super::expand_renamed_shorthand;
    use crate::ast_deobfuscate::semantic;

    /// Renames every `g` reference to `v0`.
    struct RenameG;

    impl<'a> Traverse<'a, ()> for RenameG {
        fn enter_assignment_target_property(
            &mut self,
            prop: &mut AssignmentTargetProperty<'a>,
            ctx: &mut TraverseCtx<'a, ()>,
        ) {
            expand_renamed_shorthand(prop, ctx, |binding| binding.name == "g");
        }

        fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut TraverseCtx<'a, ()>) {
            if ident.name == "g" {
                ident.name = Ident::from_str_in("v0", &ctx.ast);
            }
        }
    }

    fn rename_g(code: &str) -> String {
        let allocator = Allocator::default();
        let mut program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new((), scoping, &allocator);
        traverse_mut_with_ctx(&mut RenameG, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn keeps_the_property_key_of_a_renamed_shorthand_target() {
        assert_eq!(rename_g("({ g } = o);"), "({g: v0} = o);\n");
        assert_eq!(rename_g("({ g = 1, h } = o);"), "({g: v0 = 1, h} = o);\n");
    }
}
