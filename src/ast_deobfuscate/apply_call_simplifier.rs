//! `.apply(undefined, [...])` / `.call(undefined, ...)` → direct call simplifier.
//!
//! Rewrites:
//! - `fn.apply(undefined, [a, b, c])`  → `fn(a, b, c)`
//! - `fn.call(void 0, a, b, c)`        → `fn(a, b, c)`
//!
//! ONLY fires when:
//! 1. The first argument is `undefined` (an unresolved reference) or `void` of
//!    a literal. A `null` receiver is kept: a strict-mode callee sees `null`,
//!    not `undefined`.
//! 2. The callee is not a member expression: `o.f(a)` would bind `this` to `o`,
//!    and not `eval`: `eval(s)` would turn an indirect eval into a direct one.
//! 3. For `apply`, the 2nd argument is a literal `ArrayExpression` with no
//!    spread elements or holes.
//!
use oxc_allocator::{CloneIn, GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{Argument, ArrayExpressionElement, Expression, UnaryOperator};
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ApplyCallSimplifier {
    rewrites: usize,
}

counter_pass!(ApplyCallSimplifier, rewrites, rewrites);

fn is_undefined(expr: &Expression<'_>, ctx: &Ctx<'_>) -> bool {
    match expr {
        Expression::Identifier(id) => {
            id.name.as_str() == "undefined"
                && id
                    .reference_id
                    .get()
                    .is_some_and(|reference| ctx.scoping().get_reference(reference).symbol_id().is_none())
        }
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::Void => u.argument.is_literal(),
        _ => false,
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for ApplyCallSimplifier {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let Expression::StaticMemberExpression(sme) = &call.callee else {
            return;
        };
        let method = sme.property.name.as_str();
        if method != "apply" && method != "call" {
            return;
        }
        if call.arguments.is_empty() {
            return;
        }
        let Some(first_arg) = call.arguments[0].as_expression() else {
            return;
        };
        let callee = sme.object.without_parentheses();
        if !is_undefined(first_arg, ctx)
            || callee.is_member_expression()
            || matches!(callee, Expression::Identifier(id) if id.name == "eval")
            || matches!(callee, Expression::ChainExpression(_))
        {
            return;
        }

        let new_callee = sme.object.clone_in_with_semantic_ids(ctx.ast.allocator());
        let new_args: OxcVec<'a, Argument<'a>> = match method {
            "call" => {
                let mut v = oxc_allocator::ArenaVec::new_in(&ctx.ast);
                for arg in call.arguments.iter().skip(1) {
                    v.push(arg.clone_in_with_semantic_ids(ctx.ast.allocator()));
                }
                v
            }
            "apply" => {
                if call.arguments.len() != 2 {
                    return;
                }
                let Some(Expression::ArrayExpression(arr)) = call.arguments[1].as_expression() else {
                    return;
                };
                if arr.elements.iter().any(|e| {
                    matches!(
                        e,
                        ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
                    )
                }) {
                    return;
                }
                let mut v = oxc_allocator::ArenaVec::new_in(&ctx.ast);
                for el in &arr.elements {
                    if let Some(e) = el.as_expression() {
                        v.push(Argument::from(e.clone_in_with_semantic_ids(ctx.ast.allocator())));
                    } else {
                        return;
                    }
                }
                v
            }
            _ => return,
        };

        if self.rewrites < 10 {
            deob_trace!(
                "[AST/apply-call] rewriting .{method}({} args) -> direct call ({} args)",
                call.arguments.len(),
                new_args.len()
            );
        }
        self.rewrites += 1;
        *expr = Expression::new_call_expression(SPAN, new_callee, None, new_args, false, &ctx.ast);
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
        let mut pass = ApplyCallSimplifier::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    #[test]
    fn apply_with_undefined_and_array() {
        assert_eq!(run("fn.apply(undefined, [a, b, c]);"), "fn(a, b, c);\n");
    }

    #[test]
    fn call_with_void_zero() {
        assert_eq!(run("fn.call(void 0, a, b);"), "fn(a, b);\n");
    }

    #[test]
    fn leaves_null_receiver_alone() {
        for code in ["fn.call(null, a, b);\n", "fn.apply(null, [x]);\n"] {
            assert_eq!(run(code), code);
        }
    }

    #[test]
    fn leaves_member_callee_alone() {
        for code in [
            "v = 2 - this._x.call(undefined, node.data);\n",
            "obj.method.apply(void 0, [a, b]);\n",
            "D8()[x19()[Rj]].call(undefined, Hn, QS, fw);\n",
            "(a?.b).call(undefined, x);\n",
        ] {
            assert_eq!(run(code), code);
        }
    }

    #[test]
    fn leaves_eval_alone() {
        for code in ["eval.call(undefined, s);\n", "eval.apply(void 0, [s]);\n"] {
            assert_eq!(run(code), code);
        }
    }

    #[test]
    fn leaves_side_effecting_receiver_alone() {
        let code = "fn.call(void g(), a);\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn leaves_shadowed_undefined_alone() {
        let code = "function h(undefined) {\n\tfn.call(undefined, a);\n}\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn leaves_apply_with_non_null_this_alone() {
        let out = run("fn.apply(self, [a]);");
        assert!(out.contains(".apply"), "got: {out}");
    }

    #[test]
    fn leaves_apply_with_non_literal_args_alone() {
        let out = run("fn.apply(undefined, args);");
        assert!(out.contains(".apply"), "got: {out}");
    }

    #[test]
    fn leaves_spread_in_array_alone() {
        let out = run("fn.apply(undefined, [a, ...b]);");
        assert!(out.contains(".apply"), "got: {out}");
    }
}
