use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, BindingPattern, Expression, FormalParameters, Function, Program,
    ReturnStatement, Statement, UnaryOperator, VariableDeclaration, VariableDeclarationKind, VariableDeclarator,
};
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, SourceType};
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{Ancestor, Traverse};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::rename_guards::is_global_binding;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::ast_deobfuscate::with_scope::{redirectable_symbols, scopes_in_with_bodies};

use super::eval::ConstExpr;
use super::{Binding, Constant, Ctx, Hoisting, IndexConstants, Reach};

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

/// A read `f()[k]` that only reads the element.
struct ElementRead {
    at: u32,
    scope: ScopeId,
    index: Option<ConstExpr>,
}

/// A statement that gives an index constant its value.
struct Definition {
    binding: Binding,
    expr: ConstExpr,
    reach: Reach,
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
/// - It is not a global of a classic script, which other scripts share.
/// - It is written nowhere else, and not declared twice.
/// - No `with` body may read it as a property of the `with` object, and no
///   Annex B block function may stand in for it ([`redirectable_symbols`]).
/// - No direct `eval` can see it, since the evaluated code could assign it
///   or declare a binding that shadows it.
/// - A constant's or array's definition is a statement directly in the body
///   of the function or program that declares it, so it runs before any
///   later statement there, and only reads after its end use it ([`Reach`]).
///   A read inside a function declaration nested below that body may run
///   first, as the declaration is hoisted, so it uses the definition only
///   when no code runs before the definition.
/// - A factory that returns one shared array is only ever read as `f()[k]`,
///   never called through or deleted from, so nothing can change the array.
#[derive(Default)]
pub struct DispatchInlinerCollector {
    shapes: Vec<(SymbolId, Shape)>,
    /// Body-level `var NAME = [strings]` declarations.
    arrays: FxHashMap<SymbolId, (Vec<String>, Reach)>,
    definitions: Vec<Definition>,
    /// The `f()[k]` reads that leave the array alone, by callee reference.
    element_reads: FxHashMap<ReferenceId, ElementRead>,
    eval_scopes: Vec<ScopeId>,
    /// A `this` that may be the global object, or a `.constructor` read,
    /// which reaches `Function`.
    global_object_used: bool,
    /// Parameter bindings, which sloppy-mode `arguments[i]` also writes.
    parameters: FxHashSet<SymbolId>,
    /// Scopes of function declarations, which are hoisted.
    declaration_scopes: FxHashSet<ScopeId>,
    /// No code has run before the current program-body statement.
    in_prefix: bool,
    /// Bindings that hold a number in the program-body prefix.
    prefix_numbers: FxHashSet<Binding>,
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
    /// In the program body (`prefix`), each declarator is early while no
    /// declarator or statement before it has run code.
    fn record_declarators(&mut self, decl: &VariableDeclaration<'_>, scope: ScopeId, prefix: bool, scoping: &Scoping) {
        if !matches!(
            decl.kind,
            VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
        ) {
            self.in_prefix = false;
        }
        for declarator in &decl.declarations {
            let early = prefix && self.in_prefix;
            self.record_declarator(declarator, scope, early, scoping);
            if early {
                self.in_prefix = self.note_prefix_declarator(declarator, scoping);
            }
        }
    }

    fn record_declarator(
        &mut self,
        declarator: &VariableDeclarator<'_>,
        scope: ScopeId,
        early: bool,
        scoping: &Scoping,
    ) {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return;
        };
        let (Some(symbol), Some(init)) = (id.symbol_id.get(), &declarator.init) else {
            return;
        };
        if scoping.symbol_scope_id(symbol) != scope || self.parameters.contains(&symbol) {
            return;
        }
        let reach = Reach {
            ready: declarator.span.end,
            scope,
            early,
        };
        if let Some(strings) = Self::extract_string_array(init) {
            self.arrays.insert(symbol, (strings, reach));
        } else if let Some(expr) = ConstExpr::from_ast(init, scoping) {
            self.definitions.push(Definition {
                binding: Binding::Symbol(symbol),
                expr,
                reach,
                write: None,
            });
        }
    }

    /// Records the constant a body-level `NAME = …;` defines.
    fn record_assignment(&mut self, expr: &Expression<'_>, scope: ScopeId, prefix: bool, scoping: &Scoping) {
        let early = prefix && self.in_prefix;
        if prefix {
            self.in_prefix = false;
        }
        let Expression::AssignmentExpression(assign) = expr else {
            return;
        };
        let AssignmentTarget::AssignmentTargetIdentifier(target) = &assign.left else {
            return;
        };
        let binding = Binding::of(target, scoping);
        // A sloppy-mode parameter is also written through `arguments[i]`.
        let declared_here = match &binding {
            Binding::Symbol(symbol) => scoping.symbol_scope_id(*symbol) == scope && !self.parameters.contains(symbol),
            Binding::Global(_) => scope == scoping.root_scope_id(),
        };
        if assign.operator != AssignmentOperator::Assign || !declared_here {
            return;
        }
        let Some(value) = ConstExpr::from_ast(&assign.right, scoping) else {
            return;
        };
        if early {
            // A `let`, `const` or class binding may still be uninitialised.
            let assignable = match binding {
                Binding::Symbol(symbol) => scoping.symbol_flags(symbol).is_function_scoped_declaration(),
                Binding::Global(_) => true,
            };
            self.in_prefix = assignable && self.notes_number(binding.clone(), &value);
        }
        self.definitions.push(Definition {
            binding,
            expr: value,
            reach: Reach {
                ready: assign.span.end,
                scope,
                early,
            },
            write: Some(target.reference_id()),
        });
    }

    /// Whether a program-body declarator runs no code, noting the binding it
    /// gives a number: its value only creates a function or string array, or
    /// computes a number from literals and earlier such numbers.
    fn note_prefix_declarator(&mut self, declarator: &VariableDeclarator<'_>, scoping: &Scoping) -> bool {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return false;
        };
        let Some(init) = &declarator.init else {
            return true;
        };
        if matches!(
            init,
            Expression::FunctionExpression(_)
                | Expression::ArrowFunctionExpression(_)
                | Expression::StringLiteral(_)
                | Expression::BooleanLiteral(_)
                | Expression::NullLiteral(_)
        ) || Self::extract_string_array(init).is_some()
        {
            return true;
        }
        let (Some(symbol), Some(value)) = (id.symbol_id.get(), ConstExpr::from_ast(init, scoping)) else {
            return false;
        };
        self.notes_number(Binding::Symbol(symbol), &value)
    }

    /// Whether `value` computes a number from literals and prefix numbers,
    /// which then `binding` holds.
    fn notes_number(&mut self, binding: Binding, value: &ConstExpr) -> bool {
        let pure = value.reads_only(&self.prefix_numbers);
        if pure {
            self.prefix_numbers.insert(binding);
        }
        pure
    }

    /// Whether `this` here may be the global object: in a sloppy-mode
    /// function called without a receiver, or at the top level of a script.
    fn this_may_be_global(ctx: &Ctx<'_>) -> bool {
        let scoping = ctx.scoping();
        for scope in ctx.ancestor_scopes() {
            let flags = scoping.scope_flags(scope);
            if flags.is_arrow() {
                continue;
            }
            if flags.is_function() || flags.contains(ScopeFlags::ClassStaticBlock) {
                return !flags.is_strict_mode();
            }
        }
        ctx.ancestors()
            .last()
            .is_some_and(|program| matches!(program, Ancestor::ProgramBody(p) if p.source_type().is_script()))
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
        // Every reference but `except` is an `f()[k]` read that `reach`
        // reaches, with `k` a constant index below `len`. Any other key may
        // run an `Array.prototype` accessor that gets hold of the array.
        let only_element_reads = |symbol: SymbolId, except: Option<ReferenceId>, reach: Option<&Reach>, len: usize| {
            scoping.get_resolved_reference_ids(symbol).iter().all(|&reference| {
                Some(reference) == except
                    || self.element_reads.get(&reference).is_some_and(|read| {
                        reach.is_none_or(|reach| reach.reaches(read.at, read.scope, &self.constants.hoisting))
                            && read
                                .index
                                .as_ref()
                                .and_then(|index| {
                                    index.eval(&|binding, at, scope| self.constants.get(binding, at, scope))
                                })
                                .and_then(|index| usize::try_from(index).ok())
                                .is_some_and(|index| index < len)
                    })
            })
        };
        let mut factories = FxHashMap::default();
        for (symbol, shape) in &self.shapes {
            let symbol = *symbol;
            let strings = match shape {
                Shape::Fresh(strings) => guards.stable(symbol, None).then_some(strings),
                Shape::SelfInit(strings, write) => (guards.stable(symbol, Some(*write))
                    && only_element_reads(symbol, Some(*write), None, strings.len()))
                .then_some(strings),
                Shape::Returns(array, reference) => self.arrays.get(array).and_then(|(strings, reach)| {
                    (guards.stable(symbol, None)
                        && guards.stable(*array, None)
                        && scoping.get_resolved_reference_ids(*array) == [*reference]
                        && only_element_reads(symbol, None, Some(reach), strings.len()))
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
                    .eval(&|binding, at, scope| self.constants.get(binding, at, scope))
                    .and_then(|v| usize::try_from(v).ok());
                let Some(value) = value else {
                    pending.push(definition);
                    continue;
                };
                let constant = Constant {
                    value,
                    reach: definition.reach,
                };
                self.constants.constants.insert(definition.binding, constant);
            }
            if pending.len() == before {
                break;
            }
        }
    }
}

/// Globals that reach the global object, or run code in global scope from a
/// string, where it may assign any global.
const GLOBAL_OBJECT_NAMES: &[&str] = &[
    "globalThis",
    "window",
    "self",
    "global",
    "frames",
    "parent",
    "top",
    "eval",
    "Function",
    "setTimeout",
    "setInterval",
];

/// What can give a binding a value other than its definition.
struct Guards<'s> {
    scoping: &'s Scoping,
    /// Bindings a `with` object or an Annex B block function may stand in for.
    redirectable: FxHashSet<SymbolId>,
    /// Scopes a direct `eval` can see.
    eval_visible: FxHashSet<ScopeId>,
    /// Scopes inside `with` bodies, where a global's name may read a property.
    with_bodies: FxHashSet<ScopeId>,
    /// The program's source type: a classic script shares its globals.
    source_type: SourceType,
    /// The program names the global object or runs code in global scope, so
    /// it may assign a global without naming it.
    global_object_reachable: bool,
}

impl<'s> Guards<'s> {
    fn new(program: &Program<'_>, scoping: &'s Scoping, eval_scopes: &[ScopeId], global_object_used: bool) -> Self {
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
            source_type: program.source_type,
            global_object_reachable: global_object_used
                || GLOBAL_OBJECT_NAMES
                    .iter()
                    .any(|&name| scoping.root_unresolved_references().contains_key(name)),
        }
    }

    /// Whether `symbol` is private to the program, declared once and written
    /// by nothing but `write`.
    fn stable(&self, symbol: SymbolId, write: Option<ReferenceId>) -> bool {
        let scoping = self.scoping;
        !is_global_binding(self.source_type, scoping, symbol)
            && scoping.symbol_redeclarations(symbol).is_empty()
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
                // Other scripts share a classic script's globals, and a direct
                // eval anywhere may assign one.
                !self.source_type.is_script()
                    && !self.global_object_reachable
                    && self.eval_visible.is_empty()
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
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        // A module this one imports or re-exports runs first and may call into it.
        self.in_prefix = !program.body.iter().any(|stmt| {
            matches!(
                stmt,
                Statement::ImportDeclaration(_)
                    | Statement::ExportAllDeclaration(_)
                    | Statement::ExportFromDeclaration(_)
            )
        });
    }

    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        if !func.is_declaration() {
            return;
        }
        self.declaration_scopes.insert(func.scope_id());
        let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get()) else {
            return;
        };
        if let Some(shape) = Self::try_shape(func, symbol, ctx.scoping()) {
            self.shapes.push((symbol, shape));
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut Ctx<'a>) {
        let prefix = match ctx.parent() {
            Ancestor::ProgramBody(_) => true,
            Ancestor::FunctionBodyStatements(_) => false,
            _ => return,
        };
        let scope = ctx.current_scope_id();
        match stmt {
            Statement::VariableDeclaration(decl) => self.record_declarators(decl, scope, prefix, ctx.scoping()),
            Statement::ExpressionStatement(es) => self.record_assignment(&es.expression, scope, prefix, ctx.scoping()),
            Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => {}
            _ => {
                if prefix {
                    self.in_prefix = false;
                }
            }
        }
    }

    fn enter_formal_parameters(&mut self, params: &mut FormalParameters<'a>, _ctx: &mut Ctx<'a>) {
        let rest = params.rest.as_ref().map(|rest| &rest.rest.argument);
        for pattern in params.items.iter().map(|param| &param.pattern).chain(rest) {
            self.parameters.extend(
                pattern
                    .get_binding_identifiers()
                    .iter()
                    .filter_map(|id| id.symbol_id.get()),
            );
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut Ctx<'a>) {
        match expr {
            Expression::CallExpression(call) => {
                if matches!(call.callee.without_parentheses(), Expression::Identifier(id) if id.name == "eval") {
                    self.eval_scopes.push(ctx.current_scope_id());
                }
            }
            Expression::ThisExpression(_) => {
                self.global_object_used |= Self::this_may_be_global(ctx);
            }
            Expression::StaticMemberExpression(member) => {
                self.global_object_used |= member.property.name == "constructor";
            }
            Expression::ComputedMemberExpression(cme) => {
                self.global_object_used |=
                    matches!(&cme.expression, Expression::StringLiteral(key) if key.value == "constructor");
                if let Expression::CallExpression(call) = &cme.object
                    && call.arguments.is_empty()
                    && let Expression::Identifier(callee) = &call.callee
                    && Self::reads_element_only(ctx)
                {
                    let read = ElementRead {
                        at: callee.span().start,
                        scope: ctx.current_scope_id(),
                        index: ConstExpr::from_ast(&cme.expression, ctx.scoping()),
                    };
                    self.element_reads.insert(callee.reference_id(), read);
                }
            }
            _ => {}
        }
    }

    fn exit_program(&mut self, program: &mut Program<'a>, ctx: &mut Ctx<'a>) {
        let guards = Guards::new(program, ctx.scoping(), &self.eval_scopes, self.global_object_used);
        self.constants.hoisting = Hoisting::new(ctx.scoping(), &self.declaration_scopes);
        self.resolve_constants(&guards);
        self.resolve_factories(&guards);
    }
}
