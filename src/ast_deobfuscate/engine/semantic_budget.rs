//! Bounds the cost of `oxc_semantic` analysis before the pipeline runs.
//!
//! `SemanticBuilder` resolves each reference by walking up the scope chain
//! from the reference's scope to the scope that binds it. It hoists each `var`
//! (and each sloppy-mode block function) through every block scope between
//! the declaration and its function scope, and records the name in each of
//! those blocks. Both cost the number of scopes crossed. A program nested `n`
//! scopes deep with a reference or a `var` at every level, such as `n` nested
//! `switch` statements, therefore costs `O(n²)` time per analysis, and its
//! `var`s cost `O(n²)` memory. The pipeline analyses the program about twenty
//! times: 3000 nested switches that each declare a `var` took 8 s and 460 MB.
//!
//! One walk over the parsed program estimates those crossings. Above a
//! budget that grows linearly with the input, the pipeline does not run and
//! the input is only laid out.

use oxc_ast::ast::{
    BindingIdentifier, Function, IdentifierReference, Program, VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::{Visit, walk};
use oxc_str::Ident;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use rustc_hash::FxHashMap;
use std::cell::Cell;

/// Cost any input may have, however short: well under a second of analysis
/// over the whole pipeline.
const MIN_COST: u64 = 1 << 24;
/// Cost allowed per input byte. Real bundles and obfuscated scripts, up to
/// 10 MB, measured at most 1.6 (an Akamai Bot Manager script) and most
/// under 0.4.
const COST_PER_BYTE: u64 = 32;
/// Cost of recording a hoisted name in one block, relative to crossing one
/// scope in a lookup: an insert into a hash map that keeps the entry, measured
/// 50 times slower and about 100 bytes each.
const HOIST_WEIGHT: u64 = 64;

/// The cost budget for a `len`-byte input.
fn budget(len: usize) -> u64 {
    u64::try_from(len)
        .map_or(u64::MAX, |len| len.saturating_mul(COST_PER_BYTE))
        .max(MIN_COST)
}

/// Whether analysing `program`, parsed from `len` bytes, would cost more than
/// the pipeline may afford.
pub(super) fn exceeds_budget(program: &Program<'_>, len: usize) -> bool {
    let cost = scope_crossings(program).weighted();
    let budget = budget(len);
    if cost <= budget {
        return false;
    }
    eprintln!(
        "[DEOBFUSCATE] Input is nested too deeply to analyse (cost {cost}, budget {budget}); skipping deobfuscation"
    );
    true
}

/// The scopes `SemanticBuilder` crosses on a program, estimated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cost {
    /// Scopes crossed while resolving references.
    lookups: u64,
    /// Block scopes a hoisted name is recorded in.
    hoists: u64,
}

impl Cost {
    const fn weighted(self) -> u64 {
        self.lookups.saturating_add(self.hoists.saturating_mul(HOIST_WEIGHT))
    }
}

fn scope_crossings(program: &Program<'_>) -> Cost {
    let mut counter = ScopeCrossings::default();
    counter.visit_program(program);
    counter.cost
}

/// Tracks, in one walk, the depth of every open scope that binds each name,
/// so a reference costs the scopes between it and its innermost binding.
/// A reference that precedes its binding, such as a call to a function
/// declared further down, is counted as if it walked up to the program
/// scope.
#[derive(Default)]
struct ScopeCrossings<'a> {
    /// Number of scopes enclosing the current node.
    depth: u32,
    /// Depths of the enclosing `var` scopes (program, functions, static
    /// blocks, namespaces), innermost last.
    var_scopes: Vec<u32>,
    /// For each name, the depths of the open scopes that bind it, ascending.
    bindings: FxHashMap<Ident<'a>, Vec<u32>>,
    /// For each open scope, outermost first, the names it binds.
    scope_names: Vec<Vec<Ident<'a>>>,
    /// While visiting the names a `var` binds: the block scopes they are
    /// hoisted through.
    hoisted_through: Option<u32>,
    cost: Cost,
}

impl<'a> ScopeCrossings<'a> {
    /// Depth of the innermost `var` scope.
    fn var_depth(&self) -> u32 {
        self.var_scopes.last().copied().unwrap_or(0)
    }

    /// Records that the open scope at `depth` binds `name`.
    fn bind(&mut self, name: Ident<'a>, depth: u32) {
        let Some(names) = depth
            .checked_sub(1)
            .and_then(|index| self.scope_names.get_mut(index as usize))
        else {
            return;
        };
        names.push(name);
        let depths = self.bindings.entry(name).or_default();
        let at = depths.partition_point(|&bound| bound <= depth);
        depths.insert(at, depth);
    }

    fn hoist(&mut self, blocks: u32) {
        self.cost.hoists = self.cost.hoists.saturating_add(u64::from(blocks));
    }
}

impl<'a> Visit<'a> for ScopeCrossings<'a> {
    fn enter_scope(&mut self, flags: ScopeFlags, _scope_id: &Cell<Option<ScopeId>>) {
        self.depth += 1;
        self.scope_names.push(Vec::new());
        if flags.is_var() {
            self.var_scopes.push(self.depth);
        }
    }

    fn leave_scope(&mut self) {
        for name in self.scope_names.pop().unwrap_or_default() {
            if let Some(depths) = self.bindings.get_mut(&name) {
                depths.pop();
            }
        }
        if self.var_scopes.last() == Some(&self.depth) {
            self.var_scopes.pop();
        }
        self.depth -= 1;
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        let crossed = match self.bindings.get(&it.name).and_then(|depths| depths.last()) {
            Some(&bound) => self.depth - bound + 1,
            None => self.depth,
        };
        self.cost.lookups = self.cost.lookups.saturating_add(u64::from(crossed));
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        if let Some(blocks) = self.hoisted_through {
            self.hoist(blocks);
            self.bind(it.name, self.var_depth());
        } else {
            self.bind(it.name, self.depth);
        }
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if it.kind != VariableDeclarationKind::Var {
            walk::walk_variable_declaration(self, it);
            return;
        }
        for declarator in &it.declarations {
            self.hoisted_through = Some(self.depth - self.var_depth());
            self.visit_binding_pattern(&declarator.id);
            self.hoisted_through = None;
            if let Some(init) = &declarator.init {
                self.visit_expression(init);
            }
        }
    }

    /// Binds a declaration's name in the enclosing scope (a sloppy block
    /// function also in its `var` scope) and an expression's name in its own
    /// scope, which `walk_function` would not tell apart.
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if let Some(id) = &it.id
            && it.is_declaration()
        {
            self.bind(id.name, self.depth);
            let blocks = self.depth - self.var_depth();
            if blocks > 0 {
                self.hoist(blocks);
                self.bind(id.name, self.var_depth());
            }
        }
        self.enter_scope(flags, &it.scope_id);
        if let Some(id) = &it.id
            && !it.is_declaration()
        {
            self.bind(id.name, self.depth);
        }
        self.visit_formal_parameters(&it.params);
        if let Some(body) = &it.body {
            self.visit_function_body(body);
        }
        self.leave_scope();
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::{Cost, MIN_COST, budget, exceeds_budget, scope_crossings};

    fn on_program<T: Send>(code: &str, f: impl FnOnce(&oxc_ast::ast::Program<'_>) -> T + Send) -> T {
        crate::stack::run_on_sized_stack(code.len(), || {
            let allocator = Allocator::default();
            let program = Parser::new(&allocator, code, SourceType::cjs()).parse().program;
            f(&program)
        })
    }

    fn cost(code: &str) -> Cost {
        on_program(code, scope_crossings)
    }

    fn over_budget(code: &str) -> bool {
        on_program(code, |program| exceeds_budget(program, code.len()))
    }

    fn nested_switches(depth: usize) -> String {
        "switch (x) { case 1: var v = x; ".repeat(depth) + &"}".repeat(depth)
    }

    const fn lookups_hoists(lookups: u64, hoists: u64) -> Cost {
        Cost { lookups, hoists }
    }

    #[test]
    fn counts_lookup_depth_and_hoisted_blocks() {
        assert_eq!(cost("x;"), lookups_hoists(1, 0));
        assert_eq!(cost("{ var v = x; }"), lookups_hoists(2, 1));
        assert_eq!(cost("{ let v = x; }"), lookups_hoists(2, 0));
        assert_eq!(cost("{ { var [a, { b }] = x; } }"), lookups_hoists(3, 4));
        // A function is the `var` scope of its body.
        assert_eq!(cost("{ (function (p) { var v = p; }); }"), lookups_hoists(1, 0));
        // The innermost binding ends the lookup.
        assert_eq!(cost("let a; { let a; { { a; } } }"), lookups_hoists(3, 0));
        assert_eq!(cost("var a; function f() { { a; } }"), lookups_hoists(3, 0));
        // A sloppy block function is hoisted out of its block.
        assert_eq!(cost("{ function f() {} }"), lookups_hoists(0, 1));
        assert_eq!(cost("for (var i = 0; i < 1; i++);"), lookups_hoists(4, 1));
    }

    #[test]
    fn nested_switches_cost_quadratically() {
        let small = cost(&nested_switches(100)).weighted();
        let large = cost(&nested_switches(200)).weighted();
        assert!(large > 3 * small, "{small} -> {large}");
    }

    #[test]
    fn flat_code_costs_linearly() {
        let unit = "function f(a) { if (a) { var b = g(a); } }\n";
        let small = cost(&unit.repeat(1000));
        let large = cost(&unit.repeat(2000));
        assert_eq!(large, lookups_hoists(2 * small.lookups, 2 * small.hoists));
    }

    #[test]
    fn nested_functions_reading_their_parameters_cost_linearly() {
        let nested = |depth: usize| "(function (a) { a(); return ".repeat(depth) + "1" + &"; })".repeat(depth);
        assert_eq!(cost(&nested(2000)), lookups_hoists(2 * cost(&nested(1000)).lookups, 0));
    }

    #[test]
    fn budget_has_a_floor_and_grows_with_input() {
        assert_eq!(budget(0), MIN_COST);
        assert!(budget(1 << 30) > MIN_COST);
    }

    #[test]
    fn input_over_the_budget_is_only_laid_out() {
        let nested = |depth: usize| "switch (x) { case 1: var v = !0; ".repeat(depth) + &"}".repeat(depth);
        let deobfuscate = |code: &str| {
            crate::ast_deobfuscate::AstDeobfuscator::new()
                .deobfuscate(code)
                .expect("deobfuscates")
        };
        let shallow = deobfuscate(&nested(10));
        assert!(shallow.contains("v = true") && !shallow.contains("!0"), "{shallow}");
        let deep = deobfuscate(&nested(3000));
        assert_eq!(deep.matches("var v = !0;").count(), 3000);
    }

    #[test]
    fn deep_nesting_exceeds_the_budget_and_wide_code_does_not() {
        assert!(over_budget(&nested_switches(3000)));
        assert!(!over_budget(&nested_switches(300)));
        assert!(!over_budget(&"switch (x) { case 1: var v = x; }\n".repeat(50_000)));
    }
}
