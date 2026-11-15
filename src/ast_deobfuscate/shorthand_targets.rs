//! Shorthand properties that a rename would change.
//!
//! `({ x } = o)` and `({ x = d } = o)` spell the property key and the
//! assigned binding with one identifier. Renaming the binding in place also
//! renames the key, so the assignment would read another property. A renaming
//! pass calls [`expand_renamed_shorthand`] on every
//! [`AssignmentTargetProperty`] first, which spells the property out as
//! `({ x: x } = o)` when the binding is about to be renamed.
//!
//! The pass also calls [`expand_renamed_proto_shorthand`] on every
//! [`ObjectProperty`], for the one object literal shorthand codegen cannot
//! spell out by itself.

use oxc_allocator::{Box as ArenaBox, TakeIn};
use oxc_ast::ast::{
    AssignmentTarget, AssignmentTargetMaybeDefault, AssignmentTargetProperty, Expression, IdentifierReference,
    ObjectProperty, PropertyKey,
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

/// Turns the shorthand `{ __proto__ }` into `{ ["__proto__"]: __proto__ }`
/// when `renames` says a pass will rename the identifier. The shorthand
/// defines an own property named `__proto__`, but `{ __proto__: v0 }`, which
/// codegen would print once the names differ, sets the prototype instead.
/// Other shorthand object properties and binding patterns need nothing:
/// codegen prints `{ x: v0 }` when the key and the value differ.
pub fn expand_renamed_proto_shorthand<'a, State>(
    prop: &mut ObjectProperty<'a>,
    ctx: &TraverseCtx<'a, State>,
    renames: impl FnOnce(&IdentifierReference<'a>) -> bool,
) {
    if !prop.shorthand {
        return;
    }
    let (PropertyKey::StaticIdentifier(key), Expression::Identifier(value)) = (&prop.key, &prop.value) else {
        return;
    };
    if key.name != "__proto__" || !renames(value) {
        return;
    }
    prop.key = PropertyKey::new_string_literal(key.span, "__proto__", None, ctx);
    prop.computed = true;
    prop.shorthand = false;
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{AssignmentTargetProperty, BindingIdentifier, Ident, IdentifierReference, ObjectProperty};
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};

    use super::{expand_renamed_proto_shorthand, expand_renamed_shorthand};
    use crate::ast_deobfuscate::semantic;

    /// The name `RenameG` gives `name`: `g` becomes `v0`, `__proto__` `v1`.
    fn new_name(name: &str) -> Option<&'static str> {
        match name {
            "g" => Some("v0"),
            "__proto__" => Some("v1"),
            _ => None,
        }
    }

    /// Renames every `g` binding and reference to `v0`, and `__proto__` to `v1`.
    struct RenameG;

    impl<'a> Traverse<'a, ()> for RenameG {
        fn enter_assignment_target_property(
            &mut self,
            prop: &mut AssignmentTargetProperty<'a>,
            ctx: &mut TraverseCtx<'a, ()>,
        ) {
            expand_renamed_shorthand(prop, ctx, |binding| new_name(&binding.name).is_some());
        }

        fn enter_object_property(&mut self, prop: &mut ObjectProperty<'a>, ctx: &mut TraverseCtx<'a, ()>) {
            expand_renamed_proto_shorthand(prop, ctx, |binding| new_name(&binding.name).is_some());
        }

        fn exit_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut TraverseCtx<'a, ()>) {
            if let Some(name) = new_name(&ident.name) {
                ident.name = Ident::from_str_in(name, &ctx.ast);
            }
        }

        fn exit_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut TraverseCtx<'a, ()>) {
            if let Some(name) = new_name(&ident.name) {
                ident.name = Ident::from_str_in(name, &ctx.ast);
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
        assert_eq!(rename_g("[{ g }] = a;"), "[{g: v0}] = a;\n");
        assert_eq!(rename_g("for ({ g } of a);"), "for ({g: v0} of a);\n");
    }

    #[test]
    fn a_renamed_proto_shorthand_stays_an_own_property() {
        assert_eq!(rename_g("f({ __proto__ });"), "f({ [\"__proto__\"]: v1 });\n");
        assert_eq!(rename_g("f({ __proto__: g });"), "f({ __proto__: v0 });\n");
    }

    /// Codegen spells these out by itself, because the AST keeps their key
    /// apart from the renamed identifier.
    #[test]
    fn codegen_keeps_the_keys_of_other_renamed_shorthands() {
        assert_eq!(rename_g("f({ g, h });"), "f({\n\tg: v0,\n\th\n});\n");
        assert_eq!(rename_g("var { g, h = 1 } = o;"), "var { g: v0, h = 1 } = o;\n");
        assert_eq!(rename_g("function f({ g = 1 }) {}"), "function f({ g: v0 = 1 }) {}\n");
        assert_eq!(rename_g("var g; export { g };"), "var v0;\nexport { v0 as g };\n");
        assert_eq!(rename_g("import { g } from \"m\";"), "import { g as v0 } from \"m\";\n");
    }
}
