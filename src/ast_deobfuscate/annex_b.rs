//! Bindings that an Annex B block function may overwrite at runtime.

use oxc_ast::ast::{Function, FunctionBody, Program, Statement};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::scope_tree::ScopeTree;

/// Bindings whose references may read a sloppy-mode block function at
/// runtime, so a pass must not treat a reference to them as reaching their
/// own declaration.
///
/// Annex B.3.3 copies a sloppy-mode plain function declared in a block (or an
/// `if`/`switch`/label body) into a same-named `var` of the enclosing function
/// or script when the block runs. A reference `oxc_semantic` resolves to an
/// outer binding can therefore read the block function, and a reference it
/// resolves to the block function hoisted to the function scope can run
/// before the block. A binding is returned when it shares its name with such a
/// block function and is declared in the block that declares that function or
/// in one of its ancestor scopes; that covers the block function itself, every
/// binding a hoisted `var` would shadow, and the bindings that block Annex B
/// (where `oxc_semantic` may still hoist the function past them).
#[must_use]
pub fn annex_b_shadowed_symbols(program: &Program<'_>, scoping: &Scoping) -> FxHashSet<SymbolId> {
    let mut finder = BlockFunctions {
        scoping,
        declaring: FxHashMap::default(),
    };
    finder.visit_program(program);
    if finder.declaring.is_empty() {
        return FxHashSet::default();
    }
    let tree = ScopeTree::new(scoping);
    // Per name, the sorted tree positions of the scopes declaring one.
    let declaring: FxHashMap<&str, Vec<usize>> = finder
        .declaring
        .into_iter()
        .map(|(name, scopes)| {
            let mut positions: Vec<usize> = scopes.into_iter().map(|scope| tree.position(scope)).collect();
            positions.sort_unstable();
            (name, positions)
        })
        .collect();
    // A binding is shadowed when a same-named block function is declared in
    // its scope or a descendant: a position in its scope's subtree.
    scoping
        .symbol_ids()
        .filter(|&symbol| {
            declaring.get(scoping.symbol_name(symbol)).is_some_and(|positions| {
                let subtree = tree.subtree(scoping.symbol_scope_id(symbol));
                let first = positions.partition_point(|position| position < subtree.start());
                positions.get(first).is_some_and(|position| subtree.contains(position))
            })
        })
        .collect()
}

struct BlockFunctions<'s> {
    scoping: &'s Scoping,
    /// Per block-function name, the scopes declaring one.
    declaring: FxHashMap<&'s str, Vec<ScopeId>>,
}

impl BlockFunctions<'_> {
    /// Statements of a program or function body, where a function declaration
    /// is var-scoped without Annex B.
    fn visit_body(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            if matches!(statement, Statement::FunctionDeclaration(_)) {
                walk::walk_statement(self, statement);
            } else {
                self.visit_statement(statement);
            }
        }
    }

    fn record(&mut self, func: &Function<'_>) {
        let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get()) else {
            return;
        };
        let Some(declaring) = func
            .scope_id
            .get()
            .and_then(|scope| self.scoping.scope_parent_id(scope))
        else {
            return;
        };
        if func.r#async || func.generator || self.scoping.scope_flags(declaring).is_strict_mode() {
            return;
        }
        self.declaring
            .entry(self.scoping.symbol_name(symbol))
            .or_default()
            .push(declaring);
    }
}

impl<'a> Visit<'a> for BlockFunctions<'_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.visit_body(&program.body);
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.visit_body(&body.statements);
    }

    fn visit_statement(&mut self, statement: &Statement<'a>) {
        if let Statement::FunctionDeclaration(func) = statement {
            self.record(func);
        }
        walk::walk_statement(self, statement);
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::annex_b_shadowed_symbols;
    use crate::ast_deobfuscate::scaling::assert_near_linear;
    use crate::ast_deobfuscate::semantic;

    /// The shadowed bindings as `name@offset`, in declaration order.
    fn shadowed(code: &str) -> Vec<String> {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
        let scoping = semantic::scoping(&program);
        let symbols = annex_b_shadowed_symbols(&program, &scoping);
        let mut found: Vec<_> = symbols.into_iter().collect();
        found.sort_unstable();
        found
            .into_iter()
            .map(|symbol| {
                let start = scoping.symbol_span(symbol).start as usize;
                format!("{}@{start}", scoping.symbol_name(symbol))
            })
            .collect()
    }

    #[test]
    fn pins_outer_bindings_of_a_block_function_name() {
        let code = "function g() { function f() {} { function f() {} } return f(); }";
        let f1 = code.find("f()").unwrap();
        let f2 = code.rfind("f() {}").unwrap();
        assert_eq!(shadowed(code), [format!("f@{f1}"), format!("f@{f2}")]);
    }

    #[test]
    fn pins_hoisted_block_functions_and_bindings_in_ancestor_scopes() {
        assert_eq!(shadowed("var f; function g(x) { if (x) function f() {} }").len(), 2);
        assert_eq!(
            shadowed("function g(x) { switch (x) { case 1: function f() {} } }").len(),
            1
        );
        assert_eq!(shadowed("function g() { l: { function f() {} } }").len(), 1);
    }

    #[test]
    fn ignores_unrelated_bindings() {
        for code in [
            // Body-level declarations are var-scoped without Annex B.
            "function g() { function f() {} } function f() {}",
            // Strict, async and generator block functions stay block-scoped.
            "'use strict'; function f() {} { function f() {} }",
            "function f() {} { async function f() {} }",
            "function f() {} { function* f() {} }",
        ] {
            let pinned = shadowed(code);
            assert!(pinned.is_empty(), "{code}: {pinned:?}");
        }
        // A sibling function's own `f` is not on the block function's scope chain.
        let code = "function h() { function f() {} } function g() { { function f() {} } }";
        let inner = code.rfind("f() {}").unwrap();
        assert_eq!(shadowed(code), [format!("f@{inner}")]);
    }

    #[test]
    fn nested_blocks_with_distinct_functions_scale_linearly() {
        assert_near_linear("annex_b_shadowed_symbols", 500, |n| {
            let code = format!(
                "{}{}",
                (0..n)
                    .map(|i| format!("{{ function a_{i}() {{}} h(); var v_{i} = 1; "))
                    .collect::<Vec<_>>()
                    .concat(),
                "}".repeat(n)
            );
            let allocator = Allocator::default();
            let program = Parser::new(&allocator, &code, SourceType::cjs()).parse().program;
            let scoping = semantic::scoping(&program);
            let started = std::time::Instant::now();
            let shadowed = annex_b_shadowed_symbols(&program, &scoping);
            let elapsed = started.elapsed();
            assert_eq!(shadowed.len(), n);
            elapsed
        });
    }
}
