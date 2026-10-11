//! Reverses minifier idioms that have an equivalent, more readable form:
//!
//! - `for (; x;) s` and `for (;;) s` become `while (x) s` and `while (true) s`.
//! - `Error("x")` becomes `new Error("x")` for the unshadowed global builtins
//!   whose call and construct behaviour is specified to be identical.
//! - `!(a === b)` becomes `a !== b` (likewise `==`, `!=`, `!==`).
//! - `!!x` becomes `x` in `if`, loop, and ternary tests, where only
//!   truthiness is observed.
//! - `if (!a) X; else Y;` becomes `if (a) Y; else X;`, and
//!   `else { if (b) Z; }` becomes `else if (b) Z;`.

use oxc_allocator::TakeIn;
use oxc_ast::ast::{CallExpression, Expression, IfStatement, Statement, UnaryOperator};
use oxc_semantic::ScopeFlags;
use oxc_span::SPAN;
use oxc_traverse::{Traverse, TraverseCtx};

use crate::ast_deobfuscate::pass::counter_pass;
use crate::ast_deobfuscate::short_circuit_to_if::negated_equality;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct MinifierIdiomReverser {
    rewritten_count: usize,
}

counter_pass!(MinifierIdiomReverser, rewritten_count, rewritten_count);

impl<'a> Traverse<'a, DeobfuscateState> for MinifierIdiomReverser {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        let replacement = match expr {
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
                let negated = match unary.argument.without_parentheses() {
                    Expression::BinaryExpression(binary) => negated_equality(binary.operator),
                    _ => None,
                };
                negated.map(|operator| {
                    let mut comparison = unary.argument.without_parentheses_mut().take_in(ctx);
                    if let Expression::BinaryExpression(binary) = &mut comparison {
                        binary.operator = operator;
                    }
                    comparison
                })
            }
            Expression::CallExpression(call) if is_constructible_builtin_call(call, ctx) => {
                let callee = call.callee.take_in(ctx);
                let arguments = call.arguments.take_in(ctx);
                Some(Expression::new_new_expression(call.span, callee, None, arguments, ctx))
            }
            Expression::ConditionalExpression(cond) => {
                if strip_double_not(&mut cond.test, ctx) {
                    self.rewritten_count += 1;
                }
                None
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            *expr = replacement;
            self.rewritten_count += 1;
        }
    }

    fn exit_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        match stmt {
            Statement::ForStatement(for_stmt) if for_stmt.init.is_none() && for_stmt.update.is_none() => {
                let mut test = for_stmt
                    .test
                    .take()
                    .unwrap_or_else(|| Expression::new_boolean_literal(SPAN, true, ctx));
                strip_double_not(&mut test, ctx);
                let body = for_stmt.body.take_in(ctx);
                let span = for_stmt.span;
                *stmt = Statement::new_while_statement(span, test, body, ctx);
                self.rewritten_count += 1;
            }
            Statement::WhileStatement(while_stmt) => {
                if strip_double_not(&mut while_stmt.test, ctx) {
                    self.rewritten_count += 1;
                }
            }
            Statement::DoWhileStatement(do_while) => {
                if strip_double_not(&mut do_while.test, ctx) {
                    self.rewritten_count += 1;
                }
            }
            Statement::ForStatement(for_stmt) => {
                if let Some(test) = &mut for_stmt.test
                    && strip_double_not(test, ctx)
                {
                    self.rewritten_count += 1;
                }
            }
            Statement::IfStatement(if_stmt) => {
                if strip_double_not(&mut if_stmt.test, ctx) {
                    self.rewritten_count += 1;
                }
                if let Some(alternate) = &mut if_stmt.alternate
                    && let Some(else_if) = lone_if_in_block(alternate)
                {
                    *alternate = else_if;
                    self.rewritten_count += 1;
                }
                if flip_negated_if(if_stmt, ctx) {
                    self.rewritten_count += 1;
                }
            }
            _ => {}
        }
    }
}

/// Builtins for which `F(...)` and `new F(...)` are the same operation: their
/// spec steps replace an undefined NewTarget with the active function. `RegExp`
/// is excluded here because a call returns an existing regexp argument as is.
/// `Array(n)` and `Object(x)` are equivalent too, but those call forms are the
/// idiomatic spelling, not a minifier artifact.
const CALL_IS_CONSTRUCT: [&str; 9] = [
    "Error",
    "EvalError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "TypeError",
    "URIError",
    "AggregateError",
    "Function",
];

fn is_constructible_builtin_call(call: &CallExpression<'_>, ctx: &Ctx<'_>) -> bool {
    let Expression::Identifier(ident) = &call.callee else {
        return false;
    };
    let name = ident.name.as_str();
    let builtin = CALL_IS_CONSTRUCT.contains(&name) || (name == "RegExp" && regexp_call_is_construct(call));
    if call.optional || !builtin {
        return false;
    }
    let scoping = ctx.scoping();
    // A direct `eval` can declare a shadowing `var`, and a `with` object can
    // supply the name (and become the call's receiver).
    scoping.get_reference(ident.reference_id()).symbol_id().is_none()
        && !scoping.scope_flags(scoping.root_scope_id()).contains_direct_eval()
        && !ctx
            .ancestor_scopes()
            .any(|scope| scoping.scope_flags(scope).contains(ScopeFlags::With))
}

/// `RegExp(p)` returns `p` itself when `p` is a regexp, and `new RegExp(p)`
/// copies it, so only a primitive pattern makes the two equivalent.
fn regexp_call_is_construct(call: &CallExpression<'_>) -> bool {
    match call.arguments.first() {
        None => true,
        Some(argument) => argument.as_expression().is_some_and(|pattern| {
            matches!(
                pattern,
                Expression::StringLiteral(_)
                    | Expression::NumericLiteral(_)
                    | Expression::BooleanLiteral(_)
                    | Expression::NullLiteral(_)
                    | Expression::BigIntLiteral(_)
                    | Expression::TemplateLiteral(_)
            )
        }),
    }
}

fn strip_double_not<'a>(test: &mut Expression<'a>, ctx: &Ctx<'a>) -> bool {
    let Expression::UnaryExpression(outer) = test.without_parentheses_mut() else {
        return false;
    };
    if outer.operator != UnaryOperator::LogicalNot {
        return false;
    }
    let Expression::UnaryExpression(inner) = outer.argument.without_parentheses_mut() else {
        return false;
    };
    if inner.operator != UnaryOperator::LogicalNot {
        return false;
    }
    let value = inner.argument.take_in(ctx);
    *test = value;
    true
}

/// Flips `if (!a) X; else Y;` to `if (a) Y; else X;`. Else-if chains and
/// empty branches are left alone: flipping them does not read better.
fn flip_negated_if<'a>(if_stmt: &mut IfStatement<'a>, ctx: &Ctx<'a>) -> bool {
    let Some(alternate) = &mut if_stmt.alternate else {
        return false;
    };
    if matches!(alternate, Statement::IfStatement(_)) || is_empty(alternate) || is_empty(&if_stmt.consequent) {
        return false;
    }
    let Expression::UnaryExpression(unary) = if_stmt.test.without_parentheses_mut() else {
        return false;
    };
    if unary.operator != UnaryOperator::LogicalNot {
        return false;
    }
    let test = unary.argument.take_in(ctx);
    if_stmt.test = test;
    std::mem::swap(&mut if_stmt.consequent, alternate);
    true
}

/// `else { if (b) Y }` is `else if (b) Y`: the block holds no declarations.
/// A sloppy-mode `if (b) function f() {}` is left alone, since Annex B scopes
/// that function to an implicit block of its own.
fn lone_if_in_block<'a>(stmt: &mut Statement<'a>) -> Option<Statement<'a>> {
    let Statement::BlockStatement(block) = stmt else {
        return None;
    };
    let [Statement::IfStatement(inner)] = block.body.as_slice() else {
        return None;
    };
    let declares_function = |branch: &Statement<'_>| matches!(branch, Statement::FunctionDeclaration(_));
    if declares_function(&inner.consequent) || inner.alternate.as_ref().is_some_and(declares_function) {
        return None;
    }
    block.body.pop()
}

fn is_empty(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::EmptyStatement(_) => true,
        Statement::BlockStatement(block) => block.body.is_empty(),
        _ => false,
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

    fn run_in(code: &str, source_type: SourceType) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        assert!(
            ret.diagnostics.is_empty(),
            "test input must parse: {:?}",
            ret.diagnostics
        );
        let mut program = ret.program;
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &allocator);
        traverse_mut_with_ctx(&mut MinifierIdiomReverser::new(), &mut program, &mut ctx);
        Codegen::new().build(&program).code
    }

    fn run(code: &str) -> String {
        run_in(code, SourceType::mjs())
    }

    fn assert_unchanged(code: &str, source_type: SourceType) {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, source_type).parse().program;
        assert_eq!(run_in(code, source_type), Codegen::new().build(&program).code);
    }

    #[test]
    fn for_without_init_or_update_becomes_while() {
        assert_eq!(run("for (; x;) y();"), "while (x) y();\n");
        assert_eq!(run("for (;;) {}"), "while (true) {}\n");
        assert_eq!(
            run("l: for (;;) { continue l; }"),
            "l: while (true) {\n\tcontinue l;\n}\n"
        );
    }

    #[test]
    fn for_with_init_or_update_stays() {
        assert_unchanged("for (i = 0; x;) y();", SourceType::mjs());
        assert_unchanged("for (; x; i++) y();", SourceType::mjs());
        assert_unchanged("for (let i; x;) y();", SourceType::mjs());
    }

    #[test]
    fn restores_new_on_unshadowed_builtins() {
        assert_eq!(run("throw TypeError(\"x\");"), "throw new TypeError(\"x\");\n");
        assert_eq!(
            run("f(Error(), AggregateError([]), Function(\"a\", b));"),
            "f(new Error(), new AggregateError([]), new Function(\"a\", b));\n"
        );
        assert_eq!(
            run("RegExp(\"a\", \"g\"); RegExp();"),
            "new RegExp(\"a\", \"g\");\nnew RegExp();\n"
        );
    }

    #[test]
    fn regexp_with_non_primitive_pattern_stays_a_call() {
        // `RegExp(re)` returns `re` itself; `new RegExp(re)` returns a copy.
        assert_unchanged("RegExp(p); RegExp(/a/); RegExp(...args);", SourceType::mjs());
    }

    #[test]
    fn builtins_whose_call_differs_from_construct_stay_calls() {
        assert_unchanged(
            "Date(); Symbol(); String(x); Number(x); Boolean(x); Promise(); Array(3); Object(x);",
            SourceType::mjs(),
        );
    }

    #[test]
    fn shadowed_or_dynamic_builtin_names_stay_calls() {
        assert_unchanged("function f(Error) { return Error(\"x\"); }", SourceType::mjs());
        assert_unchanged("let TypeError = g; TypeError(1);", SourceType::mjs());
        assert_unchanged("globalThis.Error(\"x\"); Error?.(\"x\");", SourceType::mjs());
        assert_unchanged("with (o) { Error(\"x\"); }", SourceType::script());
        assert_unchanged("function f(s) { eval(s); return Error(\"x\"); }", SourceType::script());
        assert_unchanged("function f(s) { (eval)(s); } Error(\"x\");", SourceType::script());
    }

    #[test]
    fn negated_equality_flips_the_operator() {
        assert_eq!(run("x = !(a === b), y = !(a != b);"), "x = a !== b, y = a == b;\n");
    }

    #[test]
    fn negated_relational_keeps_the_negation() {
        assert_unchanged("x = !(a < b), y = !(a >= b), z = !(a in b);", SourceType::mjs());
    }

    #[test]
    fn double_negation_drops_only_in_truthiness_tests() {
        assert_eq!(run("if (!!x) y();"), "if (x) y();\n");
        assert_eq!(
            run("while (!!x) y(); do y(); while (!!x);"),
            "while (x) y();\ndo\n\ty();\nwhile (x);\n"
        );
        assert_eq!(run("z = !!x ? a : b;"), "z = x ? a : b;\n");
        assert_unchanged("z = !!x; f(!!x);", SourceType::mjs());
    }

    #[test]
    fn negated_if_with_else_swaps_branches() {
        assert_eq!(run("if (!a) b(); else c();"), "if (a) c();\nelse b();\n");
        assert_eq!(
            run("if (!(a && b)) { c(); } else { d(); }"),
            "if (a && b) {\n\td();\n} else {\n\tc();\n}\n"
        );
    }

    #[test]
    fn negated_if_without_both_branches_stays() {
        assert_unchanged("if (!a) b();", SourceType::mjs());
        assert_unchanged("if (!a) b(); else if (c) d();", SourceType::mjs());
        assert_unchanged("if (!a) {} else c();", SourceType::mjs());
        assert_unchanged("if (!a) b(); else;", SourceType::mjs());
    }

    #[test]
    fn swapped_branch_keeps_its_else_binding() {
        let out = run("if (!a) { b(); } else while (x) if (y) z();");
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, &out, SourceType::mjs()).parse().program;
        let Statement::IfStatement(outer) = &program.body[0] else {
            panic!("expected an if statement: {out}");
        };
        assert!(outer.alternate.is_some(), "the else must stay on the outer if: {out}");
    }

    #[test]
    fn else_block_with_lone_if_becomes_else_if() {
        assert_eq!(
            run("if (a) b(); else { if (c) d(); }"),
            "if (a) b();\nelse if (c) d();\n"
        );
        assert_unchanged("if (a) b(); else { if (c) d(); e(); }", SourceType::mjs());
        assert_unchanged("if (a) b(); else { if (c) function f() {} }", SourceType::script());
    }
}
