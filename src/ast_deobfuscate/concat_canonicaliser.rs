//! String `.concat` canonicaliser.
//!
//! Obfuscators and TypeScript targeting ES5 rewrite template literals as
//! `.concat` chains. This pass turns them back into template literals:
//!
//! ```text
//! "".concat(a, "=").concat(b)   ->  `${a}=${b}`
//! "prefix".concat(x)            ->  `prefix${x}`
//! a.concat(b)                   ->  (left alone: `a` is not provably a string)
//! ```
//!
//! Both forms convert each value with `ToString`, unlike `+`, which uses
//! `ToPrimitive` with the `default` hint. They differ in order: `.concat`
//! evaluates every argument before converting any, while a template converts
//! each substitution right after evaluating it. The rewrite therefore only
//! fires when at most one argument of a call is not a primitive literal, so no
//! conversion can observe or affect the evaluation of a later argument.

use oxc_allocator::{GetAllocator, TakeIn, Vec as ArenaVec};
use oxc_ast::ast::{Argument, Expression, TemplateElement};
use oxc_span::SPAN;
use oxc_traverse::{Ancestor, Traverse, TraverseCtx};

use crate::ast_deobfuscate::dyn_concat_resolver::literal::make_template_element;
use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct ConcatCanonicaliser {
    rewrites: usize,
}

counter_pass!(ConcatCanonicaliser, rewrites, rewrites);

const fn is_primitive_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
    )
}

/// Template pieces: `quasis.len() == expressions.len() + 1`.
struct Pieces<'a> {
    quasis: Vec<String>,
    expressions: Vec<Expression<'a>>,
}

/// Whether `receiver` is a string or a template whose text is known.
fn is_string_receiver(receiver: &Expression<'_>) -> bool {
    match receiver {
        Expression::StringLiteral(lit) => !lit.lone_surrogates,
        Expression::TemplateLiteral(tmpl) => tmpl
            .quasis
            .iter()
            .all(|quasi| quasi.value.cooked.is_some() && !quasi.lone_surrogates),
        _ => false,
    }
}

impl<'a> Pieces<'a> {
    /// `receiver` must satisfy [`is_string_receiver`].
    fn from_receiver(receiver: Expression<'a>) -> Self {
        match receiver {
            Expression::StringLiteral(lit) => Self {
                quasis: vec![lit.value.to_string()],
                expressions: Vec::new(),
            },
            Expression::TemplateLiteral(tmpl) => {
                let tmpl = tmpl.unbox();
                Self {
                    quasis: tmpl
                        .quasis
                        .iter()
                        .map(|quasi| quasi.value.cooked.expect("checked by is_string_receiver").to_string())
                        .collect(),
                    expressions: tmpl.expressions.into_iter().collect(),
                }
            }
            _ => unreachable!("checked by is_string_receiver"),
        }
    }

    fn push(&mut self, arg: Expression<'a>) {
        if let Expression::StringLiteral(lit) = &arg {
            self.quasis
                .last_mut()
                .expect("pieces always hold a quasi")
                .push_str(&lit.value);
        } else {
            self.expressions.push(arg);
            self.quasis.push(String::new());
        }
    }

    fn into_expression(self, ctx: &Ctx<'a>) -> Expression<'a> {
        if self.expressions.is_empty() {
            return Expression::new_string_literal(SPAN, ctx.allocator().alloc_str(&self.quasis[0]), None, ctx);
        }
        let last = self.quasis.len() - 1;
        let mut quasis: ArenaVec<'a, TemplateElement<'a>> = ArenaVec::new_in(ctx);
        for (i, quasi) in self.quasis.iter().enumerate() {
            quasis.push(make_template_element(quasi, i == last, ctx));
        }
        let mut expressions = ArenaVec::new_in(ctx);
        expressions.extend(self.expressions);
        Expression::new_template_literal(SPAN, quasis, expressions, ctx)
    }
}

fn valid_concat_arguments(arguments: &[Argument<'_>]) -> bool {
    let mut non_literals = 0;
    for arg in arguments {
        match arg.as_expression() {
            None => return false,
            Some(Expression::StringLiteral(lit)) if lit.lone_surrogates => return false,
            Some(expr) if !is_primitive_literal(expr) => non_literals += 1,
            Some(_) => {}
        }
    }
    non_literals <= 1
}

/// The receiver of `<receiver>.concat(<args>)` when the call's arguments can
/// become template pieces.
fn concat_receiver<'b, 'a>(expr: &'b Expression<'a>) -> Option<&'b Expression<'a>> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    let Expression::StaticMemberExpression(sme) = &call.callee else {
        return None;
    };
    (sme.property.name == "concat" && valid_concat_arguments(&call.arguments)).then_some(&sme.object)
}

/// Whether the current call is the receiver of an enclosing `.concat` call
/// that will absorb it.
fn is_concat_receiver(ctx: &Ctx<'_>) -> bool {
    let Ancestor::StaticMemberExpressionObject(sme) = ctx.parent() else {
        return false;
    };
    sme.property().name == "concat"
        && matches!(ctx.ancestor(1), Ancestor::CallExpressionCallee(call) if valid_concat_arguments(call.arguments()))
}

impl<'a> Traverse<'a, DeobfuscateState> for ConcatCanonicaliser {
    /// A chain `s.concat(a).concat(b)` is rewritten once, at its outermost
    /// call, with its pieces moved rather than cloned: rewriting each call
    /// would rebuild every prefix of the template, and cloning would copy
    /// nested arguments once per enclosing call.
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        if concat_receiver(expr).is_none() || is_concat_receiver(ctx) {
            return;
        }
        let mut calls = 0;
        let mut receiver: &Expression<'a> = expr;
        while let Some(inner) = concat_receiver(receiver) {
            calls += 1;
            receiver = inner;
        }
        if !is_string_receiver(receiver) {
            return;
        }

        let mut argument_lists = Vec::with_capacity(calls);
        let mut receiver = expr.take_in(ctx);
        for _ in 0..calls {
            let Expression::CallExpression(call) = receiver else {
                unreachable!("checked by concat_receiver");
            };
            let call = call.unbox();
            let Expression::StaticMemberExpression(sme) = call.callee else {
                unreachable!("checked by concat_receiver");
            };
            argument_lists.push(call.arguments);
            receiver = sme.unbox().object;
        }
        let mut pieces = Pieces::from_receiver(receiver);
        for arguments in argument_lists.into_iter().rev() {
            for arg in arguments {
                pieces.push(arg.into_expression());
            }
        }
        if self.rewrites < 10 {
            deob_trace!("[AST/concat] rewriting {calls} <str>.concat(...) call(s) -> template literal");
        }
        self.rewrites += calls;
        *expr = pieces.into_expression(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::semantic;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run(code: &str) -> String {
        run_measured(code).0
    }

    /// The output, and the arena bytes the pass allocated.
    fn run_measured(code: &str) -> (String, usize) {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let mut program = ret.program;
        let mut pass = ConcatCanonicaliser::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        let before = allocator.used_bytes();
        traverse_mut_with_ctx(&mut pass, &mut program, &mut ctx);
        let growth = allocator.used_bytes() - before;
        (Codegen::new().build(&program).code, growth)
    }

    #[test]
    fn typescript_concat_chain() {
        assert_eq!(run("x = \"\".concat(a, \"=\").concat(b);"), "x = `${a}=${b}`;\n");
    }

    #[test]
    fn prefix_concat() {
        assert_eq!(run("x = \"prefix\".concat(y);"), "x = `prefix${y}`;\n");
    }

    #[test]
    fn literal_only_concat() {
        assert_eq!(run("x = \"a\".concat(\"`${b}\\\\\");"), "x = \"a`${b}\\\\\";\n");
    }

    #[test]
    fn escapes_template_syntax_in_quasis() {
        assert_eq!(run("x = \"`\".concat(y, \"${z}\\\\\");"), "x = `\\`${y}\\${z}\\\\`;\n");
    }

    #[test]
    fn leaves_non_string_receiver_alone() {
        let code = "x = arr.concat(y, z);\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn leaves_two_non_literal_arguments_alone() {
        let code = "x = \"\".concat(a, b, c, d, e);\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn leaves_spread_alone() {
        let code = "x = \"\".concat(...a);\n";
        assert_eq!(run(code), code);
    }

    #[test]
    fn rewrites_a_chain_once() {
        assert_eq!(
            run("x = \"\".concat(a).concat(\"-\", 1).concat(b);"),
            "x = `${a}-${1}${b}`;\n"
        );
    }

    #[test]
    fn chain_stops_at_a_call_it_cannot_rewrite() {
        assert_eq!(
            run("x = \"\".concat(a).concat(b, c).concat(d);"),
            "x = `${a}`.concat(b, c).concat(d);\n"
        );
    }

    #[test]
    fn rewrites_nested_concat_arguments() {
        assert_eq!(
            run("x = \"\".concat(\"a\".concat(b)).concat(\"c\");"),
            "x = `${`a${b}`}c`;\n"
        );
    }

    /// Rewriting each call of a chain rebuilt every prefix of the template,
    /// and cloning nested arguments copied them once per enclosing call.
    #[test]
    fn long_and_nested_chains_allocate_linearly() {
        let n = 2_000;
        for code in [
            format!("x = \"\"{};", ".concat(a)".repeat(n)),
            format!("x = \"a\"{};", ".concat(\"a\")".repeat(n)),
            format!("x = {}a{};", "\"\".concat(".repeat(n), ")".repeat(n)),
        ] {
            let growth = crate::stack::run_on_sized_stack(code.len(), || run_measured(&code).1);
            assert!(
                growth <= 64 * code.len(),
                "{growth} bytes for {} source bytes",
                code.len()
            );
        }
    }
}
