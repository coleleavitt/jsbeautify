use oxc_allocator::Vec as OxcVec;
use oxc_ast::ast::{
    BindingIdentifier, BindingPattern, Expression, Function, Program, ReturnStatement, Statement, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_span::SPAN;
use oxc_syntax::node::NodeId;
use oxc_traverse::Traverse;
use std::cell::Cell;

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
        use oxc_ast::ast::{ArrayExpression, CallExpression, NewExpression, NullLiteral, ObjectExpression};
        match kind {
            InitKind::EmptyArray => Expression::ArrayExpression(ctx.ast.alloc(ArrayExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                elements: ctx.ast.vec(),
            })),
            InitKind::EmptyObject => Expression::ObjectExpression(ctx.ast.alloc(ObjectExpression {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                properties: ctx.ast.vec(),
            })),
            InitKind::NewObject | InitKind::NewArray => {
                let name = if matches!(kind, InitKind::NewObject) {
                    "Object"
                } else {
                    "Array"
                };
                let callee = Expression::Identifier(ctx.ast.alloc(oxc_ast::ast::IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(name),
                    reference_id: Cell::default(),
                }));
                Expression::NewExpression(ctx.ast.alloc(NewExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee,
                    arguments: ctx.ast.vec(),
                    type_arguments: None,
                    pure: false,
                }))
            }
            InitKind::ObjectCreateNull | InitKind::ObjectCreateProto => {
                let obj = Expression::Identifier(ctx.ast.alloc(oxc_ast::ast::IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident("Object"),
                    reference_id: Cell::default(),
                }));
                let property = oxc_ast::ast::IdentifierName {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident("create"),
                };
                let callee = Expression::StaticMemberExpression(ctx.ast.alloc(oxc_ast::ast::StaticMemberExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    object: obj,
                    property,
                    optional: false,
                }));
                let arg_expr = if matches!(kind, InitKind::ObjectCreateNull) {
                    Expression::NullLiteral(ctx.ast.alloc(NullLiteral {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                    }))
                } else {
                    Expression::ObjectExpression(ctx.ast.alloc(ObjectExpression {
                        node_id: Cell::new(NodeId::DUMMY),
                        span: SPAN,
                        properties: ctx.ast.vec(),
                    }))
                };
                let mut args = ctx.ast.vec();
                args.push(oxc_ast::ast::Argument::from(arg_expr));
                Expression::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee,
                    arguments: args,
                    optional: false,
                    type_arguments: None,
                    pure: false,
                }))
            }
            InitKind::ArrayIter(method) => {
                let empty_arr = Expression::ArrayExpression(ctx.ast.alloc(ArrayExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    elements: ctx.ast.vec(),
                }));
                let property = oxc_ast::ast::IdentifierName {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(method),
                };
                let callee = Expression::StaticMemberExpression(ctx.ast.alloc(oxc_ast::ast::StaticMemberExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    object: empty_arr,
                    property,
                    optional: false,
                }));
                Expression::CallExpression(ctx.ast.alloc(CallExpression {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    callee,
                    arguments: ctx.ast.vec(),
                    optional: false,
                    type_arguments: None,
                    pure: false,
                }))
            }
        }
    }

    fn try_rewrite_function<'a>(func: &mut Function<'a>, ctx: &Ctx<'a>) -> Option<(String, InitKind)> {
        let (name, kind) = is_self_init_accessor_named(func)?;
        let cache_var = format!("__{name}_cache");

        let body = func.body.as_mut()?;
        let mut new_stmts = ctx.ast.vec();
        let return_stmt = Statement::ReturnStatement(ctx.ast.alloc(ReturnStatement {
            node_id: Cell::new(NodeId::DUMMY),
            span: SPAN,
            argument: Some(Expression::Identifier(ctx.ast.alloc(
                oxc_ast::ast::IdentifierReference {
                    node_id: Cell::new(NodeId::DUMMY),
                    span: SPAN,
                    name: ctx.ast.ident(&cache_var),
                    reference_id: Cell::default(),
                },
            ))),
        }));
        new_stmts.push(return_stmt);
        body.statements = new_stmts;
        Some((cache_var, kind))
    }

    fn hoist_cache_vars<'a>(&mut self, statements: &mut OxcVec<'a, Statement<'a>>, ctx: &Ctx<'a>) {
        if self.cache_vars_to_hoist.is_empty() {
            return;
        }
        let mut new_decls = ctx.ast.vec();
        for (name, kind) in self.cache_vars_to_hoist.drain(..) {
            let id = BindingIdentifier {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                name: ctx.ast.ident(&name),
                symbol_id: Cell::default(),
            };
            let init = Self::make_init_expr(kind, ctx);
            let declarator = VariableDeclarator {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                kind: VariableDeclarationKind::Var,
                id: BindingPattern::BindingIdentifier(ctx.ast.alloc(id)),
                type_annotation: None,
                init: Some(init),
                definite: false,
            };
            let mut declarations = ctx.ast.vec();
            declarations.push(declarator);
            let decl = Statement::VariableDeclaration(ctx.ast.alloc(VariableDeclaration {
                node_id: Cell::new(NodeId::DUMMY),
                span: SPAN,
                kind: VariableDeclarationKind::Var,
                declarations,
                declare: false,
            }));
            new_decls.push(decl);
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
