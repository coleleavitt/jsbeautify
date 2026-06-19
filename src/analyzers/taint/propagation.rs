use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression, Expression, Function, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};

use super::expr::expr_is_tainted;

/// Pass 0 — map each *named* function to its ordered parameter names, so a
/// tainted call argument can taint the matching callee parameter
/// (inter-procedural taint).
#[derive(Default)]
pub(super) struct FunctionParams {
    /// function name → ordered parameter identifier names.
    pub(super) map: HashMap<String, Vec<String>>,
}

impl FunctionParams {
    fn record(&mut self, name: &str, params: &oxc_ast::ast::FormalParameters<'_>) {
        let names: Vec<String> = params
            .items
            .iter()
            .filter_map(|p| match &p.pattern {
                BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
                _ => None,
            })
            .collect();
        if !names.is_empty() {
            self.map.insert(name.to_string(), names);
        }
    }
}

impl<'a> Visit<'a> for FunctionParams {
    fn visit_function(&mut self, func: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        if let Some(id) = &func.id {
            self.record(id.name.as_str(), &func.params);
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        // `const f = (a, b) => …` / `const f = function (a, b) {}`.
        if let BindingPattern::BindingIdentifier(id) = &decl.id {
            match &decl.init {
                Some(Expression::ArrowFunctionExpression(arrow)) => self.record(id.name.as_str(), &arrow.params),
                Some(Expression::FunctionExpression(f)) => self.record(id.name.as_str(), &f.params),
                _ => {}
            }
        }
        walk::walk_variable_declarator(self, decl);
    }
}

/// Inter-procedural pass — for every call `f(args…)` where an argument is
/// tainted, taint `f`'s corresponding parameter name so the next fixpoint round
/// propagates taint through `f`'s body.
pub(super) struct CallPropagator<'a, 'p> {
    pub(super) params: &'p HashMap<String, Vec<String>>,
    pub(super) tainted: &'a mut HashSet<String>,
}

impl<'a> Visit<'a> for CallPropagator<'a, '_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        // Only resolve a plain identifier callee `f(...)`. A method call
        // `obj.f(...)` is NOT matched by the bare method name: without scope
        // resolution we can't know it refers to a standalone `function f`, and
        // assuming so produces false positives (e.g. tainting an unrelated
        // top-level `render` because some object also has a `render` method).
        if let Expression::Identifier(callee) = &call.callee
            && let Some(param_names) = self.params.get(callee.name.as_str())
        {
            for (i, arg) in call.arguments.iter().enumerate() {
                if let Some(expr) = arg.as_expression()
                    && expr_is_tainted(expr, self.tainted)
                    && let Some(pname) = param_names.get(i)
                {
                    self.tainted.insert(pname.clone());
                }
            }
        }
        walk::walk_call_expression(self, call);
    }
}

/// Intra-procedural pass — collects tainted binding names.
pub(super) struct TaintCollector<'a> {
    pub(super) tainted: &'a mut HashSet<String>,
}

impl<'a> Visit<'a> for TaintCollector<'a> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (Some(init), BindingPattern::BindingIdentifier(id)) = (&decl.init, &decl.id)
            && expr_is_tainted(init, self.tainted)
        {
            self.tainted.insert(id.name.to_string());
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_assignment_expression(&mut self, assign: &AssignmentExpression<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left
            && expr_is_tainted(&assign.right, self.tainted)
        {
            self.tainted.insert(id.name.to_string());
        }
        walk::walk_assignment_expression(self, assign);
    }
}
