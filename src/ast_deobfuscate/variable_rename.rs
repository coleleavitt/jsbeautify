use oxc_ast::ast::{BindingIdentifier, Function, Ident, IdentifierReference};
use oxc_traverse::{Traverse, TraverseCtx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

pub struct VariableRenamer {
    rename_map: FxHashMap<String, String>,
    var_counter: usize,
    func_counter: usize,
    changed: bool,
}

impl VariableRenamer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rename_map: FxHashMap::default(),
            var_counter: 0,
            func_counter: 0,
            changed: false,
        }
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

    fn generate_name(&mut self, var_type: &VarType) -> String {
        match var_type {
            VarType::Function => {
                self.func_counter += 1;
                format!("func_{}", self.func_counter)
            }
            VarType::Variable => {
                self.var_counter += 1;
                format!("var_{}", self.var_counter)
            }
        }
    }

    fn get_or_create_rename(&mut self, old_name: &str, var_type: &VarType) -> String {
        if let Some(new_name) = self.rename_map.get(old_name) {
            new_name.clone()
        } else {
            let new_name = self.generate_name(var_type);
            self.rename_map.insert(old_name.to_string(), new_name.clone());
            new_name
        }
    }
}

enum VarType {
    Variable,
    Function,
}

impl Default for VariableRenamer {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for VariableRenamer {
    fn enter_binding_identifier(&mut self, ident: &mut BindingIdentifier<'a>, ctx: &mut Ctx<'a>) {
        let old_name = ident.name.as_str();
        if Self::should_rename(old_name) {
            let new_name = self.get_or_create_rename(old_name, &VarType::Variable);
            if crate::ast_deobfuscate::ast_trace_enabled() {
                eprintln!("[AST] Renaming binding: {old_name} → {new_name}");
            }
            ident.name = Ident::from_str_in(&new_name, ctx);
            self.changed = true;
        }
    }

    fn enter_identifier_reference(&mut self, ident: &mut IdentifierReference<'a>, ctx: &mut Ctx<'a>) {
        let old_name = ident.name.as_str();
        if let Some(new_name) = self.rename_map.get(old_name) {
            if crate::ast_deobfuscate::ast_trace_enabled() {
                eprintln!("[AST] Renaming reference: {old_name} → {new_name}");
            }
            ident.name = Ident::from_str_in(new_name, ctx);
            self.changed = true;
        }
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some(ident) = &mut func.id {
            let old_name = ident.name.as_str();
            if Self::should_rename(old_name) {
                let new_name = self.get_or_create_rename(old_name, &VarType::Function);
                if crate::ast_deobfuscate::ast_trace_enabled() {
                    eprintln!("[AST] Renaming function: {old_name} → {new_name}");
                }
                ident.name = Ident::from_str_in(&new_name, ctx);
                self.changed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast_deobfuscate::DeobfuscateState;
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_traverse::{ReusableTraverseCtx, traverse_mut_with_ctx};

    fn run_rename(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let ret = Parser::new(&allocator, code, source_type).parse();
        let mut program = ret.program;

        let mut renamer = VariableRenamer::new();
        let state = DeobfuscateState::new();
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
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
}
