use oxc_ast::ast::{BindingIdentifier, Ident, IdentifierReference};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_symbol;
use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Renames `_0x…` obfuscator bindings to `var_N` / `func_N`, by resolved
/// symbol, leaving classic-script globals and unresolved names untouched.
/// A counter value whose name any binding or free reference already uses is
/// skipped, so a new name never captures or shadows an existing one.
pub struct VariableRenamer {
    rename_map: FxHashMap<SymbolId, String>,
    /// Bindings that keep their name (`--keep-fn-names`).
    kept: FxHashSet<SymbolId>,
    /// Every binding and unresolved name in the program, read on first use.
    taken: Option<FxHashSet<String>>,
    var_counter: usize,
    func_counter: usize,
    changed: bool,
}

impl VariableRenamer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rename_map: FxHashMap::default(),
            kept: FxHashSet::default(),
            taken: None,
            var_counter: 0,
            func_counter: 0,
            changed: false,
        }
    }

    /// Never rename `symbols`, which must be ids of the program the next
    /// traversal visits.
    pub fn keep(&mut self, symbols: FxHashSet<SymbolId>) {
        self.kept = symbols;
    }

    #[must_use]
    pub const fn has_changed(&self) -> bool {
        self.changed
    }

    fn should_rename(name: &str) -> bool {
        if name.starts_with('_') && name.len() > 1 {
            let rest = &name[1..];
            if rest.starts_with("0x") {
                return true;
            }
            if rest.chars().all(|c| c.is_ascii_hexdigit()) {
                return true;
            }
        }
        false
    }

    /// The new name of `symbol`, assigned on first sight in source order.
    fn rename_for(&mut self, symbol: SymbolId, ctx: &Ctx<'_>) -> Option<String> {
        if let Some(new_name) = self.rename_map.get(&symbol) {
            return Some(new_name.clone());
        }
        if !Self::should_rename(ctx.scoping().symbol_name(symbol))
            || is_global_symbol(ctx, symbol)
            || self.kept.contains(&symbol)
        {
            return None;
        }
        let scoping = ctx.scoping();
        let taken = self.taken.get_or_insert_with(|| {
            scoping
                .symbol_names()
                .map(str::to_string)
                .chain(
                    scoping
                        .root_unresolved_references()
                        .keys()
                        .map(|name| name.as_str().to_string()),
                )
                .collect()
        });
        let (prefix, counter) = if scoping.symbol_flags(symbol).is_function() {
            ("func", &mut self.func_counter)
        } else {
            ("var", &mut self.var_counter)
        };
        let new_name = loop {
            *counter += 1;
            let candidate = format!("{prefix}_{counter}");
            if !taken.contains(&candidate) {
                break candidate;
            }
        };
        self.rename_map.insert(symbol, new_name.clone());
        Some(new_name)
    }
}

impl Default for VariableRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for VariableRenamer {
    fn enter_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        if let Some(symbol) = ident.symbol_id.get()
            && let Some(new_name) = self.rename_for(symbol, ctx)
        {
            ident.name = Ident::from_str_in(&new_name, ctx);
            self.changed = true;
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        if let Some(symbol) = ctx.scoping().get_reference(ident.reference_id()).symbol_id()
            && let Some(new_name) = self.rename_for(symbol, ctx)
        {
            ident.name = Ident::from_str_in(&new_name, ctx);
            self.changed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::{DeobfuscateState, semantic};
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_rename(code: &str) -> String {
        run_rename_as(code, SourceType::mjs())
    }

    fn run_rename_as(code: &str, source_type: SourceType) -> String {
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut renamer = VariableRenamer::new();
        let state = DeobfuscateState::new();
        let scoping = semantic::scoping(&program);
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);

        traverse_mut_with_ctx(&mut renamer, &mut program, &mut ctx);

        Codegen::new().build(&program).code
    }

    #[test]
    fn test_rename_hex_var() {
        let code = "var _0x1234 = 42; console.log(_0x1234);";
        let output = run_rename(code);
        eprintln!("Output: {output}");

        assert!(output.contains("var_1"));
        assert!(!output.contains("_0x1234"));
    }

    #[test]
    fn test_rename_hex_function() {
        let code = "function _0xabcd() { return 42; } _0xabcd();";
        let output = run_rename(code);
        eprintln!("Output: {output}");

        assert!(output.contains("func_1"));
        assert!(!output.contains("_0xabcd"));
    }

    #[test]
    fn test_no_rename_normal_var() {
        let code = "var normalVar = 42; console.log(normalVar);";
        let output = run_rename(code);
        eprintln!("Output: {output}");

        assert!(output.contains("normalVar"));
        assert!(!output.contains("var_1"));
    }

    #[test]
    fn test_rename_multiple_vars() {
        let code = "var _0x1234 = 1; var _0x5678 = 2; console.log(_0x1234 + _0x5678);";
        let output = run_rename(code);
        eprintln!("Output: {output}");

        assert!(output.contains("var_1"));
        assert!(output.contains("var_2"));
        assert!(!output.contains("_0x"));
    }

    #[test]
    fn new_names_skip_existing_bindings_and_globals() {
        let output = run_rename("var var_1 = 'user'; var _0x1f = 'obf'; console.log(var_1, _0x1f, var_2);");
        assert!(output.contains("var var_1 = \"user\""), "{output}");
        assert!(output.contains("var var_3 = \"obf\""), "{output}");
        assert!(output.contains("console.log(var_1, var_3, var_2)"), "{output}");
    }

    #[test]
    fn hoisted_reference_before_declaration_is_renamed() {
        let output = run_rename("_0xabcd(); function _0xabcd() { return 42; }");
        assert!(!output.contains("_0xabcd"), "{output}");
    }

    #[test]
    fn unresolved_names_are_not_renamed() {
        let output = run_rename("function f(_0x1) { return _0x1 + _0x2; }");
        assert!(output.contains("_0x2"), "{output}");
        assert!(!output.contains("_0x1"), "{output}");
    }

    #[test]
    fn script_globals_are_not_renamed() {
        let code = "var _0x1 = 1; function _0x2() { var _0x1 = 2; return _0x1; } window._0x1; _0x2(_0x1);";
        let output = run_rename_as(code, SourceType::script());
        assert!(output.contains("var _0x1 = 1"), "{output}");
        assert!(output.contains("function _0x2()"), "{output}");
        assert!(output.contains("_0x2(_0x1)"), "{output}");
        assert!(
            output.contains("var var_1 = 2"),
            "shadowing local is still renamed: {output}"
        );
    }

    #[test]
    fn commonjs_top_level_is_renamed() {
        let output = run_rename_as("var _0x1 = 1; f(_0x1);", SourceType::cjs());
        assert!(!output.contains("_0x1"), "{output}");
    }
}
