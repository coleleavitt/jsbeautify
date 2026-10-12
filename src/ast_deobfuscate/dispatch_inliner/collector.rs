use oxc_ast::ast::{AssignmentTarget, BindingPattern, Expression, Function, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::eval::{ConstExpr, try_eval_as_usize};

pub struct DispatchInlinerCollector {
    factories: FxHashMap<String, Vec<String>>,
    constants: FxHashMap<String, usize>,
    unresolved: Vec<(String, ConstExpr)>,
    hoisted_arrays: FxHashMap<String, Vec<String>>,
}

impl DispatchInlinerCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            factories: FxHashMap::default(),
            constants: FxHashMap::default(),
            unresolved: Vec::new(),
            hoisted_arrays: FxHashMap::default(),
        }
    }

    #[must_use]
    pub fn into_maps(mut self) -> (FxHashMap<String, Vec<String>>, FxHashMap<String, usize>) {
        // Multi-pass: resolve constants that depend on other constants
        for _ in 0..10 {
            let prev_len = self.constants.len();
            self.unresolved.retain(|(name, expr)| {
                if let Some(v) = expr.eval(&self.constants)
                    && let Ok(u) = usize::try_from(v)
                {
                    self.constants.insert(name.clone(), u);
                    return false;
                }
                true
            });
            if self.constants.len() == prev_len {
                break;
            }
        }
        (self.factories, self.constants)
    }

    fn extract_string_array(expr: &Expression<'_>) -> Option<Vec<String>> {
        let Expression::ArrayExpression(arr) = expr else {
            return None;
        };
        let mut strings = Vec::with_capacity(arr.elements.len());
        for elem in &arr.elements {
            let expr = elem.as_expression()?;
            let Expression::StringLiteral(s) = expr else {
                return None;
            };
            strings.push(s.value.as_str().to_string());
        }
        Some(strings)
    }

    fn try_direct_return(func: &Function<'_>) -> Option<Vec<String>> {
        let body = func.body.as_ref()?;
        if body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return None;
        };
        let arg = ret.argument.as_ref()?;
        Self::extract_string_array(arg)
    }

    fn try_self_init_accessor(func: &Function<'_>) -> Option<Vec<String>> {
        let func_name = func.id.as_ref()?.name.as_str();
        let body = func.body.as_ref()?;
        if body.statements.len() != 3 {
            return None;
        }

        let Statement::VariableDeclaration(var_decl) = &body.statements[0] else {
            return None;
        };
        if var_decl.declarations.len() != 1 {
            return None;
        }
        let decl = &var_decl.declarations[0];
        let BindingPattern::BindingIdentifier(local_id) = &decl.id else {
            return None;
        };
        let local_name = local_id.name.as_str();
        let init = decl.init.as_ref()?;
        let strings = Self::extract_string_array(init)?;

        let Statement::ExpressionStatement(expr_stmt) = &body.statements[1] else {
            return None;
        };
        let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
            return None;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target_id) = &assign.left else {
            return None;
        };
        if target_id.name.as_str() != func_name {
            return None;
        }
        let Expression::FunctionExpression(inner_fn) = &assign.right else {
            return None;
        };
        let inner_body = inner_fn.body.as_ref()?;
        if inner_body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(inner_ret) = &inner_body.statements[0] else {
            return None;
        };
        let Some(Expression::Identifier(ret_id)) = &inner_ret.argument else {
            return None;
        };
        if ret_id.name.as_str() != local_name {
            return None;
        }

        let Statement::ReturnStatement(ret) = &body.statements[2] else {
            return None;
        };
        let Some(Expression::Identifier(ret_id2)) = &ret.argument else {
            return None;
        };
        if ret_id2.name.as_str() != local_name {
            return None;
        }

        Some(strings)
    }

    fn try_extract_factory(func: &Function<'_>) -> Option<Vec<String>> {
        Self::try_direct_return(func).or_else(|| Self::try_self_init_accessor(func))
    }

    fn try_return_ident(func: &Function<'_>) -> Option<String> {
        let body = func.body.as_ref()?;
        if body.statements.len() != 1 {
            return None;
        }
        let Statement::ReturnStatement(ret) = &body.statements[0] else {
            return None;
        };
        let Some(Expression::Identifier(ident)) = &ret.argument else {
            return None;
        };
        Some(ident.name.as_str().to_string())
    }
}

impl Default for DispatchInlinerCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatchInlinerCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, _ctx: &mut Ctx<'a>) {
        if let Some(id) = &func.id {
            let name = id.name.as_str().to_string();
            let strings = Self::try_extract_factory(func).or_else(|| {
                let ident = Self::try_return_ident(func)?;
                self.hoisted_arrays.get(&ident).cloned()
            });
            if let Some(strings) = strings {
                deob_trace!(
                    "[AST/dispatch-inline] found string-array factory {}() with {} elements",
                    name,
                    strings.len()
                );
                self.factories.insert(name, strings);
            }
        }
    }

    fn enter_variable_declarator(&mut self, decl: &mut VariableDeclarator<'a>, _ctx: &mut Ctx<'a>) {
        let BindingPattern::BindingIdentifier(id) = &decl.id else {
            return;
        };
        let Some(init) = &decl.init else {
            return;
        };
        let name = id.name.as_str().to_string();
        if let Some(strings) = Self::extract_string_array(init) {
            self.hoisted_arrays.insert(name.clone(), strings);
        }
        if let Some(val) = try_eval_as_usize(init, &self.constants) {
            self.constants.insert(name, val);
        } else if let Some(ce) = ConstExpr::from_ast(init) {
            self.unresolved.push((name, ce));
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, _ctx: &mut Ctx<'a>) {
        let Statement::ExpressionStatement(es) = stmt else {
            return;
        };
        let Expression::AssignmentExpression(assign) = &es.expression else {
            return;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return;
        };
        let name = target.name.as_str().to_string();
        if let Some(val) = try_eval_as_usize(&assign.right, &self.constants) {
            self.constants.insert(name, val);
        } else if let Some(ce) = ConstExpr::from_ast(&assign.right) {
            self.unresolved.push((name, ce));
        }
    }
}
