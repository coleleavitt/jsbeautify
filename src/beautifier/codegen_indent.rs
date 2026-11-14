//! Indentation of the codegen text a [`Layout`](super::Layout) lays out.
//!
//! Codegen indents every line by its nesting depth and the layout reads that
//! depth back from the leading tabs, so text nested `d` levels deep costs `d`
//! bytes per line: a statement inside 50,000 blocks (100 KB of input) became
//! 2.5 GB of tabs before the layout multiplied it by the indent size. Past
//! [`MAX_INDENT_DEPTH`] levels the text is generated without indentation, so
//! its size stays linear in the input; the layout then indents only its own
//! adjustments.

use oxc_allocator::Vec as ArenaVec;
use oxc_ast::AstKind;
use oxc_ast::ast::{Argument, CallExpression, IfStatement, NewExpression, Program};
use oxc_ast_visit::Visit;
use oxc_codegen::CodegenOptions;

/// Deepest nesting that is laid out with indentation.
pub const MAX_INDENT_DEPTH: usize = 128;

/// Codegen options for text the layout will lay out: indented as
/// [`CodegenOptions::default`] unless `program` nests deeper than
/// [`MAX_INDENT_DEPTH`].
pub fn layout_codegen_options(program: &Program<'_>) -> CodegenOptions {
    let mut nesting = Nesting::default();
    nesting.visit_program(program);
    if nesting.max <= MAX_INDENT_DEPTH {
        return CodegenOptions::default();
    }
    eprintln!("[beautify] nesting depth exceeds {MAX_INDENT_DEPTH} levels; output is not indented by nesting");
    unindented_codegen_options()
}

/// Codegen options without indentation, for text that is only re-parsed.
pub fn unindented_codegen_options() -> CodegenOptions {
    CodegenOptions {
        indent_width: 0,
        ..CodegenOptions::default()
    }
}

/// Upper bound on the indentation levels codegen emits: every node that can
/// open an indented line counts one level. Chains that codegen prints at one
/// level (`else if`, call and member chains, operators) do not count.
#[derive(Default)]
struct Nesting {
    depth: usize,
    max: usize,
}

impl Nesting {
    fn nested(&mut self, visit: impl FnOnce(&mut Self)) {
        self.depth += 1;
        self.max = self.max.max(self.depth);
        visit(self);
        self.depth -= 1;
    }

    fn visit_call_arguments<'a>(&mut self, arguments: &ArenaVec<'a, Argument<'a>>) {
        if !arguments.is_empty() {
            self.nested(|nesting| nesting.visit_arguments(arguments));
        }
    }
}

const fn indents(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::BlockStatement(_)
            | AstKind::FunctionBody(_)
            | AstKind::StaticBlock(_)
            | AstKind::ClassBody(_)
            | AstKind::SwitchStatement(_)
            | AstKind::SwitchCase(_)
            | AstKind::ObjectExpression(_)
            | AstKind::ObjectPattern(_)
            | AstKind::ArrayExpression(_)
            | AstKind::ArrayPattern(_)
            | AstKind::ForStatement(_)
            | AstKind::ForInStatement(_)
            | AstKind::ForOfStatement(_)
            | AstKind::WhileStatement(_)
            | AstKind::DoWhileStatement(_)
            | AstKind::WithStatement(_)
            | AstKind::LabeledStatement(_)
            | AstKind::ImportExpression(_)
            | AstKind::JSXElement(_)
            | AstKind::JSXFragment(_)
            | AstKind::TSInterfaceBody(_)
            | AstKind::TSTypeLiteral(_)
            | AstKind::TSEnumBody(_)
            | AstKind::TSModuleBlock(_)
            | AstKind::TSTypeParameterDeclaration(_)
    )
}

impl<'a> Visit<'a> for Nesting {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        if indents(kind) {
            self.depth += 1;
            self.max = self.max.max(self.depth);
        }
    }

    fn leave_node(&mut self, kind: AstKind<'a>) {
        if indents(kind) {
            self.depth -= 1;
        }
    }

    /// Codegen braces a nested `if` (`if (a) { if (b) … }`), but prints an
    /// `else if` chain at the level of its first `if`.
    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.nested(|nesting| {
            nesting.visit_expression(&it.test);
            nesting.visit_statement(&it.consequent);
        });
        if let Some(alternate) = &it.alternate {
            self.visit_statement(alternate);
        }
    }

    /// Arguments are indented when they carry comments; a call chain is not.
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        self.visit_expression(&it.callee);
        self.visit_call_arguments(&it.arguments);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        self.visit_expression(&it.callee);
        self.visit_call_arguments(&it.arguments);
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::*;

    fn indent_width(code: &str) -> usize {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        layout_codegen_options(&parsed.program).indent_width
    }

    fn blocks(depth: usize) -> String {
        format!("{}x();{}", "{".repeat(depth), "}".repeat(depth))
    }

    #[test]
    fn indents_up_to_the_limit() {
        let indented = CodegenOptions::default().indent_width;
        assert_ne!(indented, 0);
        assert_eq!(indent_width(&blocks(MAX_INDENT_DEPTH)), indented);
        assert_eq!(indent_width(&blocks(MAX_INDENT_DEPTH + 1)), 0);
    }

    #[test]
    fn counts_the_braces_codegen_adds_to_nested_ifs() {
        let depth = MAX_INDENT_DEPTH + 1;
        assert_eq!(indent_width(&format!("{}x();", "if (a) ".repeat(depth))), 0);
        assert_eq!(
            indent_width(&format!("f({}0{});", "f(".repeat(depth), ")".repeat(depth))),
            0
        );
        assert_eq!(
            indent_width(&format!("x = {}0{};", "{a: ".repeat(depth), "}".repeat(depth))),
            0
        );
    }

    #[test]
    fn chains_printed_at_one_level_keep_their_indentation() {
        let indented = CodegenOptions::default().indent_width;
        let length = 10 * MAX_INDENT_DEPTH;
        for chain in [
            format!("if (a) b();{}", " else if (a) b();".repeat(length)),
            format!("a{};", ".b()".repeat(length)),
            format!("x = a{};", " + a".repeat(length)),
            format!("x = {}0;", "a ? b : ".repeat(length)),
        ] {
            assert_eq!(indent_width(&chain), indented, "{}", &chain[..40]);
        }
    }

    #[test]
    fn unindented_text_has_no_leading_whitespace() {
        let allocator = Allocator::default();
        let program = Parser::new(
            &allocator,
            "function f() { if (a) { return { b: [1] }; } }",
            SourceType::mjs(),
        )
        .parse()
        .program;
        let code = oxc_codegen::Codegen::new()
            .with_options(unindented_codegen_options())
            .build(&program)
            .code;
        assert!(code.lines().count() > 3, "{code}");
        assert!(code.lines().all(|line| !line.starts_with([' ', '\t'])), "{code}");
    }
}
