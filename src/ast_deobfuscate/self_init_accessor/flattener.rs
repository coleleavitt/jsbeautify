use oxc_allocator::{GetAllocator, Vec as OxcVec};
use oxc_ast::ast::{
    BindingPattern, CallExpression, Expression, Function, IdentifierName, Program, ReturnStatement, Statement,
    VariableDeclaration, VariableDeclarationKind, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_span::{SPAN, SourceType};
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::{redirectable_symbols, scopes_in_with_bodies};

use super::Ctx;
use super::detect::Resolver;
use super::types::InitKind;

#[derive(Default)]
pub struct SelfInitAccessorFlattener {
    rewritten: usize,
    /// Rewritten accessors, with the cache variable to declare at the start of
    /// the statement list that declares each.
    pending: FxHashMap<SymbolId, (String, InitKind)>,
    /// Every name the program binds or reads, and the cache names taken.
    used_names: FxHashSet<String>,
    /// Per cache base name, the next number to try.
    next_suffix: FxHashMap<String, usize>,
    /// References that are the callee of a plain call `F()`.
    callees: FxHashSet<ReferenceId>,
    redirectable: FxHashSet<SymbolId>,
    with_scopes: FxHashSet<ScopeId>,
    has_imports: bool,
    source_type: SourceType,
}

impl SelfInitAccessorFlattener {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Whether `func`, a declaration of `accessor`, may be flattened: it is
    /// declared at the top level of a function body or the program, where its
    /// cache can be set once per run of that body before any code runs, and
    /// every reference to it is a plain call or its own self-reassignment, so
    /// no code holds the original function to call it again.
    fn is_flattenable(&self, accessor: SymbolId, parent: &Ancestor<'_, '_>, scoping: &Scoping) -> bool {
        let declared_at_body_start = match parent {
            Ancestor::ProgramBody(_) => !self.has_imports,
            Ancestor::FunctionBodyStatements(_) => true,
            _ => false,
        };
        let mut writes = 0;
        let only_called = scoping.get_resolved_reference_ids(accessor).iter().all(|&id| {
            let reference = scoping.get_reference(id);
            if reference.is_write() {
                writes += 1;
                !reference.is_read()
            } else {
                self.callees.contains(&id)
            }
        });
        declared_at_body_start
            && only_called
            && writes == 1
            && scoping.symbol_redeclarations(accessor).is_empty()
            && !self.redirectable.contains(&accessor)
            && !self.with_scopes.contains(&scoping.symbol_scope_id(accessor))
            && !is_global_binding(self.source_type, scoping, accessor)
    }

    /// `__NAME_cache`, numbered when the program already uses that name.
    fn cache_name(&mut self, accessor: &str) -> String {
        let base = format!("__{accessor}_cache");
        // Resume after the last number this base got, so `k` accessors of
        // one name cost `O(k)`, not `O(k^2)`.
        let n = self.next_suffix.entry(base.clone()).or_insert(1);
        let mut name = if *n == 1 { base.clone() } else { format!("{base}_{n}") };
        while self.used_names.contains(&name) {
            *n += 1;
            name = format!("{base}_{n}");
        }
        *n += 1;
        self.used_names.insert(name.clone());
        name
    }

    fn declare_caches<'a>(&mut self, statements: &mut OxcVec<'a, Statement<'a>>, ctx: &Ctx<'a>) {
        let caches: Vec<(String, InitKind)> = statements
            .iter()
            .filter_map(|stmt| match stmt {
                Statement::FunctionDeclaration(func) => func.id.as_ref()?.symbol_id.get(),
                _ => None,
            })
            .filter_map(|symbol| self.pending.remove(&symbol))
            .collect();
        if caches.is_empty() {
            return;
        }
        let mut new_stmts = OxcVec::with_capacity_in(caches.len() + statements.len(), ctx);
        for (name, kind) in caches {
            let name = ctx.allocator().alloc_str(&name);
            let id = BindingPattern::new_binding_identifier(SPAN, name, ctx);
            let init = Self::make_init_expr(kind, ctx);
            let declarator = VariableDeclarator::new(SPAN, id, None, Some(init), false, ctx);
            let declaration = VariableDeclaration::boxed(SPAN, VariableDeclarationKind::Var, [declarator], false, ctx);
            new_stmts.push(Statement::VariableDeclaration(declaration));
        }
        new_stmts.extend(statements.drain(..));
        *statements = new_stmts;
    }
}

/// The references that are the callee of a plain call.
#[derive(Default)]
struct Callees(FxHashSet<ReferenceId>);

impl<'a> Visit<'a> for Callees {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &call.callee
            && let Some(reference) = callee.reference_id.get()
        {
            self.0.insert(reference);
        }
        walk::walk_call_expression(self, call);
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for SelfInitAccessorFlattener {
    fn enter_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let scoping = ctx.scoping();
        self.redirectable = redirectable_symbols(program, scoping);
        self.with_scopes = scopes_in_with_bodies(scoping);
        self.has_imports = program
            .body
            .iter()
            .any(|stmt| matches!(stmt, Statement::ImportDeclaration(_)));
        self.used_names = scoping
            .symbol_names()
            .chain(scoping.root_unresolved_references().keys().map(oxc_str::Ident::as_str))
            .map(str::to_string)
            .collect();
        let mut callees = Callees::default();
        callees.visit_program(program);
        self.callees = callees.0;
        self.source_type = program.source_type;
    }

    fn exit_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if !func.is_declaration() {
            return;
        }
        let Some(accessor) = func.id.as_ref().and_then(|id| id.symbol_id.get()) else {
            return;
        };
        let scoping = ctx.scoping();
        let resolver = Resolver {
            scoping,
            with_scopes: &self.with_scopes,
        };
        let Some(kind) = resolver.self_init_accessor(func) else {
            return;
        };
        if !self.is_flattenable(accessor, &ctx.parent(), scoping) {
            return;
        }
        let accessor_name = scoping.symbol_name(accessor).to_string();
        let cache = self.cache_name(&accessor_name);
        if self.rewritten < 10 {
            deob_trace!("[AST/self-init] flattened accessor {accessor_name} (init={kind:?}, cache={cache})");
        }
        let Some(body) = func.body.as_mut() else {
            return;
        };
        let cache_ident = Expression::new_identifier(SPAN, ctx.allocator().alloc_str(&cache), ctx);
        let return_stmt = Statement::ReturnStatement(ReturnStatement::boxed(SPAN, Some(cache_ident), ctx));
        body.statements = OxcVec::from_iter_in([return_stmt], ctx);
        self.rewritten += 1;
        self.pending.insert(accessor, (cache, kind));
    }

    fn exit_statements(&mut self, statements: &mut OxcVec<'a, Statement<'a>>, ctx: &mut Ctx<'a>) {
        if !self.pending.is_empty() {
            self.declare_caches(statements, ctx);
        }
    }
}
