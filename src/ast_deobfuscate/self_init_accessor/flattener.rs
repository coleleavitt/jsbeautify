use oxc_allocator::{GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    BindingPattern, Expression, Function, IdentifierName, Program, ReturnStatement, Statement, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_traverse::Traverse;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::Ctx;
use super::detect::is_self_init_accessor_named;
use super::types::InitKind;

pub struct SelfInitAccessorFlattener {
    rewritten: usize,
    cache_vars_to_hoist: Vec<(String, InitKind)>,
}

impl SelfInitAccessorFlattener {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rewritten: 0,
            cache_vars_to_hoist: Vec::new(),
        }
    }

    #[must_use]
    pub const fn rewritten(&self) -> usize {
        self.rewritten
    }

    fn make_init_expr<'a>(kind: InitKind, ctx: &Ctx<'a>) -> Expression<'a> {
        match kind {
            InitKind::EmptyArray => Expression::new_array_expression(SPAN, OxcVec::new_in(ctx), ctx),
            InitKind::EmptyObject => Expression::new_object_expression(SPAN, OxcVec::new_in(ctx), ctx),
            InitKind::NewObject | InitKind::NewArray => {
                let name = if matches!(kind, InitKind::NewObject) {
                    "Object"
                } else {
                    "Array"
                };
                let callee = Expression::new_identifier(SPAN, name, ctx);
                Expression::new_new_expression(SPAN, callee, None, OxcVec::new_in(ctx), ctx)
            }
            InitKind::ObjectCreateNull | InitKind::ObjectCreateProto => {
                let object = Expression::new_identifier(SPAN, "Object", ctx);
                let property = IdentifierName::new(SPAN, "create", ctx);
                let callee = Expression::new_static_member_expression(SPAN, object, property, false, ctx);
                let argument = if matches!(kind, InitKind::ObjectCreateNull) {
                    Expression::new_null_literal(SPAN, ctx)
                } else {
                    Expression::new_object_expression(SPAN, OxcVec::new_in(ctx), ctx)
                };
                Expression::new_call_expression(
                    SPAN,
                    callee,
                    None,
                    [oxc_ast::ast::Argument::from(argument)],
                    false,
                    ctx,
                )
            }
            InitKind::ArrayIter(method) => {
                let object = Expression::new_array_expression(SPAN, OxcVec::new_in(ctx), ctx);
                let method = ctx.allocator().alloc_str(method);
                let property = IdentifierName::new(SPAN, method, ctx);
                let callee = Expression::new_static_member_expression(SPAN, object, property, false, ctx);
                Expression::new_call_expression(SPAN, callee, None, OxcVec::new_in(ctx), false, ctx)
            }
        }
    }

    fn try_rewrite_function<'a>(func: &mut Function<'a>, ctx: &Ctx<'a>) -> Option<(String, InitKind)> {
        let (name, kind) = is_self_init_accessor_named(func)?;
        let cache_var = format!("__{name}_cache");

        let body = func.body.as_mut()?;
        let mut new_stmts = OxcVec::new_in(ctx);
        let cache_name = ctx.allocator().alloc_str(&cache_var);
        let argument = Expression::new_identifier(SPAN, cache_name, ctx);
        let return_stmt = Statement::ReturnStatement(ReturnStatement::boxed(SPAN, Some(argument), ctx));
        new_stmts.push(return_stmt);
        body.statements = new_stmts;
        Some((cache_var, kind))
    }

    fn hoist_cache_vars<'a>(&mut self, statements: &mut OxcVec<'a, Statement<'a>>, ctx: &Ctx<'a>) {
        if self.cache_vars_to_hoist.is_empty() {
            return;
        }
        let mut new_decls = OxcVec::new_in(ctx);
        for (name, kind) in self.cache_vars_to_hoist.drain(..) {
            let name = ctx.allocator().alloc_str(&name);
            let id = BindingPattern::new_binding_identifier(SPAN, name, ctx);
            let init = Self::make_init_expr(kind, ctx);
            let declarator = VariableDeclarator::new(SPAN, id, None, Some(init), false, ctx);
            let declaration = VariableDeclaration::boxed(SPAN, VariableDeclarationKind::Var, [declarator], false, ctx);
            new_decls.push(Statement::VariableDeclaration(declaration));
        }
        for existing in statements.drain(..) {
            new_decls.push(existing);
        }
        *statements = new_decls;
    }
}

impl Default for SelfInitAccessorFlattener {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SelfInitAccessorFlattener {
    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if let Some((cache_var, kind)) = Self::try_rewrite_function(func, ctx) {
            if self.rewritten < 10 {
                eprintln!(
                    "[AST/self-init] flattened accessor {} (init={kind:?}, cache={cache_var})",
                    func.id.as_ref().map_or("?", |id| id.name.as_str()),
                );
            }
            self.rewritten += 1;
            self.cache_vars_to_hoist.push((cache_var, kind));
        }
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        self.hoist_cache_vars(&mut program.body, ctx);
    }
}
