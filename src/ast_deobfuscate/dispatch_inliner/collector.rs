use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BindingPattern, Expression, FormalParameters, Function, Program,
    ReturnStatement, Statement, UnaryOperator, VariableDeclaration,
};
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::{redirectable_symbols, scopes_in_with_bodies};

use super::eval::ConstExpr;
use super::{Binding, Constant, Ctx, IndexConstants};

/// How a factory function produces its array.
enum Shape {
    /// `return [..]`: every call returns a new array.
    Fresh(Vec<String>),
    /// The self-init accessor: every call returns one shared array. The
    /// accessor's assignment to its own binding is the binding's one write.
    SelfInit(Vec<String>, ReferenceId),
    /// `return ARRAY`, read through this reference: every call returns the
    /// array a `var ARRAY = [..]` declaration holds.
    Returns(SymbolId, ReferenceId),
}

/// A statement that gives an index constant its value.
struct Definition {
    binding: Binding,
    expr: ConstExpr,
    ready: u32,
    /// The assignment `NAME = …` that defines the binding, or `None` for a
    /// declarator `var NAME = …`.
    write: Option<ReferenceId>,
}

/// Finds string-array factories and index constants.
///
/// Both are bindings, so a match is a `SymbolId` (or, for an undeclared
/// global, its unresolved references), and a binding is used only when its
/// definition is the one way it gets a value:
///
/// - It is written nowhere else, and not declared twice.
/// - No `with` body may read it as a property of the `with` object, and no
///   Annex B block function may stand in for it ([`redirectable_symbols`]).
/// - No direct `eval` can see it, since the evaluated code could assign it
///   or declare a binding that shadows it.
/// - A constant's or array's definition is a statement directly in the body
///   of the function or program that declares it, so it runs before any
///   later statement there, and only reads after its end use it.
/// - A factory that returns one shared array is only ever read as `f()[k]`,
///   never called through or deleted from, so nothing can change the array.
#[derive(Default)]
pub struct DispatchInlinerCollector {
    shapes: Vec<(SymbolId, Shape)>,
    /// Body-level `var NAME = [strings]` declarations, with their ends.
    arrays: FxHashMap<SymbolId, (Vec<String>, u32)>,
    definitions: Vec<Definition>,
    /// Callees of the `f()[k]` reads that leave the array alone, with offsets.
    element_reads: FxHashMap<ReferenceId, u32>,
    eval_scopes: Vec<ScopeId>,
    factories: FxHashMap<SymbolId, Vec<String>>,
    constants: IndexConstants,
}

impl DispatchInlinerCollector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_maps(self) -> (FxHashMap<SymbolId, Vec<String>>, IndexConstants) {
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
            strings.push(s.value.as_str()?.to_string());
        }
        Some(strings)
    }

    /// The symbol `expr` reads, when it is an identifier bound in the program.
    fn read_symbol(expr: Option<&Expression<'_>>, scoping: &Scoping) -> Option<SymbolId> {
        let Some(Expression::Identifier(id)) = expr else {
            return None;
        };
        scoping.get_reference(id.reference_id()).symbol_id()
    }

    /// A call with no arguments binds every parameter to `undefined` (a
    /// rest parameter to `[]`) without running any code.
    fn binds_quietly(params: &FormalParameters<'_>) -> bool {
        params
            .items
            .iter()
            .all(|param| param.initializer.is_none() && matches!(param.pattern, BindingPattern::BindingIdentifier(_)))
            && params
                .rest
                .as_ref()
                .is_none_or(|rest| matches!(rest.rest.argument, BindingPattern::BindingIdentifier(_)))
    }

    /// The function's body when it is one `return` statement.
    fn single_return<'f, 'a>(func: &'f Function<'a>) -> Option<&'f ReturnStatement<'a>> {
        let body = func.body.as_ref()?;
        let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else {
            return None;
        };
        Some(ret)
    }

    fn try_self_init_accessor(func: &Function<'_>, symbol: SymbolId, scoping: &Scoping) -> Option<Shape> {
        let body = func.body.as_ref()?;
        let [
            Statement::VariableDeclaration(var_decl),
            Statement::ExpressionStatement(expr_stmt),
            Statement::ReturnStatement(ret),
        ] = body.statements.as_slice()
        else {
            return None;
        };
        let [decl] = var_decl.declarations.as_slice() else {
            return None;
        };
        let BindingPattern::BindingIdentifier(local_id) = &decl.id else {
            return None;
        };
        let local = local_id.symbol_id.get()?;
        let strings = Self::extract_string_array(decl.init.as_ref()?)?;

        let Expression::AssignmentExpression(assign) = &expr_stmt.expression else {
            return None;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return None;
        };
        if assign.operator != AssignmentOperator::Assign
            || scoping.get_reference(target.reference_id()).symbol_id() != Some(symbol)
        {
            return None;
        }
        let Expression::FunctionExpression(inner_fn) = &assign.right else {
            return None;
        };
        if Self::read_symbol(Self::single_return(inner_fn)?.argument.as_ref(), scoping) != Some(local)
            || Self::read_symbol(ret.argument.as_ref(), scoping) != Some(local)
        {
            return None;
        }
        Some(Shape::SelfInit(strings, target.reference_id()))
    }

    fn try_shape(func: &Function<'_>, symbol: SymbolId, scoping: &Scoping) -> Option<Shape> {
        if func.r#async || func.generator || !Self::binds_quietly(&func.params) {
            return None;
        }
        if let Some(ret) = Self::single_return(func) {
            return match ret.argument.as_ref()? {
                Expression::Identifier(id) => {
                    let array = scoping.get_reference(id.reference_id()).symbol_id()?;
                    Some(Shape::Returns(array, id.reference_id()))
                }
                argument => Self::extract_string_array(argument).map(Shape::Fresh),
            };
        }
        Self::try_self_init_accessor(func, symbol, scoping)
    }

    /// Records the arrays and constants a body-level declaration defines.
    fn record_declarators(&mut self, decl: &VariableDeclaration<'_>, scope: ScopeId, scoping: &Scoping) {
        for declarator in &decl.declarations {
            let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                continue;
            };
            let (Some(symbol), Some(init)) = (id.symbol_id.get(), &declarator.init) else {
                continue;
            };
            if scoping.symbol_scope_id(symbol) != scope {
                continue;
            }
            let ready = declarator.span.end;
            if let Some(strings) = Self::extract_string_array(init) {
                self.arrays.insert(symbol, (strings, ready));
            } else if let Some(expr) = ConstExpr::from_ast(init, scoping) {
                self.definitions.push(Definition {
                    binding: Binding::Symbol(symbol),
                    expr,
                    ready,
                    write: None,
                });
            }
        }
    }

    /// Records the constant a body-level `NAME = …;` defines.
    fn record_assignment(&mut self, expr: &Expression<'_>, scope: ScopeId, scoping: &Scoping) {
        let Expression::AssignmentExpression(assign) = expr else {
            return;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return;
        };
        let binding = Binding::of(target, scoping);
        let declared_here = match &binding {
            Binding::Symbol(symbol) => scoping.symbol_scope_id(*symbol) == scope,
            Binding::Global(_) => scope == scoping.root_scope_id(),
        };
        if assign.operator != AssignmentOperator::Assign || !declared_here {
            return;
        }
        if let Some(expr) = ConstExpr::from_ast(&assign.right, scoping) {
            self.definitions.push(Definition {
                binding,
                expr,
                ready: assign.span.end,
                write: Some(target.reference_id()),
            });
        }
    }

    /// Whether an `f()[k]` read whose member expression has these ancestors
    /// only reads the element.
    fn reads_element_only(ctx: &Ctx<'_>) -> bool {
        let parent = ctx
            .ancestors()
            .find(|ancestor| !matches!(ancestor, Ancestor::ParenthesizedExpressionExpression(_)));
        match parent {
            Some(Ancestor::UnaryExpressionArgument(unary)) => *unary.operator() != UnaryOperator::Delete,
            Some(
                Ancestor::CallExpressionCallee(_)
                | Ancestor::NewExpressionCallee(_)
                | Ancestor::TaggedTemplateExpressionTag(_),
            ) => false,
            _ => true,
        }
    }

    fn resolve_factories(&mut self, guards: &Guards<'_>) {
        let scoping = guards.scoping;
        // Every reference but `except` is an `f()[k]` read at or after `ready`.
        let only_element_reads = |symbol: SymbolId, except: Option<ReferenceId>, ready: u32| {
            scoping.get_resolved_reference_ids(symbol).iter().all(|&reference| {
                Some(reference) == except || self.element_reads.get(&reference).is_some_and(|&at| at >= ready)
            })
        };
        let mut factories = FxHashMap::default();
        for (symbol, shape) in &self.shapes {
            let symbol = *symbol;
            let strings = match shape {
                Shape::Fresh(strings) => guards.stable(symbol, None).then_some(strings),
                Shape::SelfInit(strings, write) => (guards.stable(symbol, Some(*write))
                    && only_element_reads(symbol, Some(*write), 0))
                .then_some(strings),
                Shape::Returns(array, reference) => self.arrays.get(array).and_then(|(strings, ready)| {
                    (guards.stable(symbol, None)
                        && guards.stable(*array, None)
                        && scoping.get_resolved_reference_ids(*array) == [*reference]
                        && only_element_reads(symbol, None, *ready))
                    .then_some(strings)
                }),
            };
            if let Some(strings) = strings {
                deob_trace!(
                    "[AST/dispatch-inline] found string-array factory {}() with {} elements",
                    scoping.symbol_name(symbol),
                    strings.len()
                );
                factories.insert(symbol, strings.clone());
            }
        }
        self.factories = factories;
    }

    fn resolve_constants(&mut self, guards: &Guards<'_>) {
        let mut defined: FxHashMap<&Binding, usize> = FxHashMap::default();
        for definition in &self.definitions {
            *defined.entry(&definition.binding).or_default() += 1;
        }
        let valid: Vec<bool> = self
            .definitions
            .iter()
            .map(|definition| defined.get(&definition.binding) == Some(&1) && guards.defines(definition))
            .collect();
        let mut pending: Vec<Definition> = std::mem::take(&mut self.definitions)
            .into_iter()
            .zip(valid)
            .filter_map(|(definition, valid)| valid.then_some(definition))
            .collect();

        // Constants may depend on later ones; a few passes settle most chains.
        for _ in 0..10 {
            let before = pending.len();
            for definition in std::mem::take(&mut pending) {
                let value = definition
                    .expr
                    .eval(&|binding, at| self.constants.get(binding, at))
                    .and_then(|v| usize::try_from(v).ok());
                let Some(value) = value else {
                    pending.push(definition);
                    continue;
                };
                let constant = Constant {
                    value,
                    ready: definition.ready,
                };
                self.constants.0.insert(definition.binding, constant);
            }
            if pending.len() == before {
                break;
            }
        }
    }
}

/// What can give a binding a value other than its definition.
struct Guards<'s> {
    scoping: &'s Scoping,
    /// Bindings a `with` object or an Annex B block function may stand in for.
    redirectable: FxHashSet<SymbolId>,
    /// Scopes a direct `eval` can see.
    eval_visible: FxHashSet<ScopeId>,
    /// Scopes inside `with` bodies, where a global's name may read a property.
    with_bodies: FxHashSet<ScopeId>,
}

impl<'s> Guards<'s> {
    fn new(program: &Program<'_>, scoping: &'s Scoping, eval_scopes: &[ScopeId]) -> Self {
        let mut eval_visible = FxHashSet::default();
        for &scope in eval_scopes {
            for ancestor in scoping.scope_ancestors(scope) {
                if !eval_visible.insert(ancestor) {
                    break;
                }
            }
        }
        Self {
            scoping,
            redirectable: redirectable_symbols(program, scoping),
            eval_visible,
            with_bodies: scopes_in_with_bodies(scoping),
        }
    }

    /// Whether `symbol` is declared once and written by nothing but `write`.
    fn stable(&self, symbol: SymbolId, write: Option<ReferenceId>) -> bool {
        let scoping = self.scoping;
        scoping.symbol_redeclarations(symbol).is_empty()
            && !self.redirectable.contains(&symbol)
            && !self.eval_visible.contains(&scoping.symbol_scope_id(symbol))
            && scoping
                .get_resolved_reference_ids(symbol)
                .iter()
                .all(|&reference| !scoping.get_reference(reference).is_write() || Some(reference) == write)
    }

    /// Whether `definition` is the only way its binding gets a value.
    fn defines(&self, definition: &Definition) -> bool {
        match &definition.binding {
            Binding::Symbol(symbol) => self.stable(*symbol, definition.write),
            Binding::Global(name) => {
                // A direct eval anywhere may assign the global.
                self.eval_visible.is_empty()
                    && self
                        .scoping
                        .root_unresolved_references()
                        .get(name.as_str())
                        .is_some_and(|references| {
                            references.iter().all(|&reference| {
                                let reference_data = self.scoping.get_reference(reference);
                                (!reference_data.is_write() || Some(reference) == definition.write)
                                    && !self.with_bodies.contains(&reference_data.scope_id())
                            })
                        })
            }
        }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatchInlinerCollector {
    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if !func.is_declaration() {
            return;
        }
        let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get()) else {
            return;
        };
        if let Some(shape) = Self::try_shape(func, symbol, ctx.scoping()) {
            self.shapes.push((symbol, shape));
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        if !matches!(
            ctx.parent(),
            Ancestor::ProgramBody(_) | Ancestor::FunctionBodyStatements(_)
        ) {
            return;
        }
        let scope = ctx.current_scope_id();
        if let Statement::VariableDeclaration(decl) = stmt {
            self.record_declarators(decl, scope, ctx.scoping());
        } else if let Statement::ExpressionStatement(es) = stmt {
            self.record_assignment(&es.expression, scope, ctx.scoping());
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        match expr {
            Expression::CallExpression(call) => {
                if matches!(call.callee.without_parentheses(), Expression::Identifier(id) if id.name == "eval") {
                    self.eval_scopes.push(ctx.current_scope_id());
                }
            }
            Expression::ComputedMemberExpression(cme) => {
                if let Expression::CallExpression(call) = &cme.object
                    && call.arguments.is_empty()
                    && let Expression::Identifier(callee) = &call.callee
                    && Self::reads_element_only(ctx)
                {
                    self.element_reads.insert(callee.reference_id(), callee.span().start);
                }
            }
            _ => {}
        }
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let guards = Guards::new(program, ctx.scoping(), &self.eval_scopes);
        self.resolve_factories(&guards);
        self.resolve_constants(&guards);
    }
}
