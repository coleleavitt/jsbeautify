//! Finds the string array, its decoders, and the rotation IIFE, proves which
//! decoder calls and element reads see a known string, and plans the rewrite.

use oxc_ast::ast::{
    AccessorProperty, ArrayExpressionElement, ArrowFunctionExpression, BindingPattern, CallExpression, ChainElement,
    ComputedMemberExpression, Expression, ExpressionStatement, FormalParameters, Function, FunctionBody,
    IdentifierReference, NewExpression, Program, PropertyDefinition, Statement, StaticBlock, StaticMemberExpression,
    SwitchCase, TaggedTemplateExpression, ThisExpression, UnaryExpression, UnaryOperator, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator, WithStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::checksum::{array_index, index_literal};
use super::decoding::{Decoding, Helpers};
use super::shape::{
    Decoder, DecoderCandidate, StringArray, decoder_candidate, match_array_variable, match_decoder, match_rotation,
    match_wrapper, reference_symbol,
};
use crate::ast_deobfuscate::annex_b::annex_b_shadowed_symbols;
use crate::ast_deobfuscate::webpack::shapes::simple_params;

/// Unbound names that refer to the global object in browsers and Node.
const GLOBAL_OBJECT_NAMES: [&str; 7] = ["globalThis", "window", "self", "global", "frames", "parent", "top"];
/// Upper bound on the string arrays analysed per body; each costs one walk.
const MAX_ARRAYS: usize = 8;
/// Walks of the whole source allowed per program.
const WALKS_PER_SOURCE_BYTE: usize = 16;
/// Upper bound on the decoders of one string array.
const MAX_DECODERS: usize = 8;
/// Checksum operations evaluated per program while solving rotations.
const MAX_ROTATION_WORK: usize = 50_000_000;
/// Inlined bytes allowed per source byte, so that inlining one long string
/// at many call sites cannot exhaust memory.
const INLINE_BYTES_PER_SOURCE_BYTE: usize = 4;
/// Inlined bytes always allowed, however short the source.
const MIN_INLINE_BYTES: usize = 1 << 20;

#[derive(Default)]
pub(super) struct Plan {
    /// The string each replaced expression reads: a decoder call, by its
    /// callee's reference, or an element read `a[N]`, by the reference to `a`.
    pub(super) replacements: FxHashMap<ReferenceId, String>,
    /// Addresses of the rotation IIFE statements to remove.
    pub(super) rotations: Vec<usize>,
    /// Decoder aliases whose every reference is replaced.
    pub(super) dead_aliases: FxHashSet<SymbolId>,
}

/// Work shared by every string array of a program.
struct Budget {
    /// Source bytes still allowed to be walked, one walk per array.
    walk_bytes: usize,
    /// Bytes still allowed to be inlined.
    inline_bytes: usize,
    /// Checksum operations still allowed while solving rotations.
    rotation_work: usize,
}

pub(super) fn address<T>(node: &T) -> usize {
    std::ptr::from_ref(node).addr()
}

pub(super) fn analyze(program: &Program<'_>, scoping: &Scoping) -> Plan {
    let mut plan = Plan::default();
    if scoping.root_scope_flags().contains_direct_eval() {
        return plan;
    }
    // In a classic script the decoder is a global, which indirect eval or
    // `Function` code can overwrite by name.
    let unresolved = scoping.root_unresolved_references();
    if program.source_type.is_script() && (unresolved.contains_key("eval") || unresolved.contains_key("Function")) {
        return plan;
    }
    let source_len = program.source_text.len();
    let mut analysis = Analysis {
        program,
        scoping,
        shadowed: annex_b_shadowed_symbols(program, scoping),
        budget: Budget {
            inline_bytes: source_len
                .saturating_mul(INLINE_BYTES_PER_SOURCE_BYTE)
                .max(MIN_INLINE_BYTES),
            rotation_work: MAX_ROTATION_WORK,
            walk_bytes: source_len.saturating_mul(WALKS_PER_SOURCE_BYTE),
        },
        plan: &mut plan,
    };
    analysis.analyze_body(&Body {
        statements: &program.body,
        scope: scoping.root_scope_id(),
        function: None,
    });
    analysis.visit_program(program);
    plan
}

/// A statement list whose leading declarations a string array may use: the
/// program, or a function body.
struct Body<'p, 'a> {
    statements: &'p [Statement<'a>],
    /// The scope its `var`s and function declarations belong to.
    scope: ScopeId,
    /// The function body; `None` for the program.
    function: Option<&'p FunctionBody<'a>>,
}

/// Analyses the program body and every function body.
struct Analysis<'s, 'p, 'a> {
    program: &'p Program<'a>,
    scoping: &'s Scoping,
    shadowed: FxHashSet<SymbolId>,
    budget: Budget,
    plan: &'s mut Plan,
}

impl<'a> Analysis<'_, '_, 'a> {
    /// Analyses the body of a function. Parameter expressions run before the
    /// body and cannot see its declarations, and inside a `with` body a
    /// global name such as `atob` may read the object's property, so such a
    /// function is left alone.
    fn analyze_function(&mut self, params: &FormalParameters<'a>, body: &FunctionBody<'a>, scope: Option<ScopeId>) {
        let scoping = self.scoping;
        if let Some(scope) = scope
            && simple_params(params).is_some()
            && !scoping
                .scope_ancestors(scope)
                .any(|ancestor| scoping.scope_flags(ancestor).is_with())
        {
            self.analyze_body(&Body {
                statements: &body.statements,
                scope,
                function: Some(body),
            });
        }
    }

    fn analyze_body(&mut self, body: &Body<'_, 'a>) {
        let mut arrays = body
            .statements
            .iter()
            .enumerate()
            .filter_map(|(index, stmt)| match stmt {
                Statement::FunctionDeclaration(func) => match_wrapper(func, self.scoping),
                Statement::VariableDeclaration(decl) => match_array_variable(decl, index),
                _ => None,
            })
            .take(MAX_ARRAYS)
            .peekable();
        if arrays.peek().is_none() {
            return;
        }
        let candidates: Vec<DecoderCandidate<'_, '_>> = body
            .statements
            .iter()
            .enumerate()
            .filter_map(|(index, stmt)| decoder_candidate(stmt, index))
            .collect();
        let helpers = Helpers::new(&candidates);
        for array in arrays {
            self.analyze_array(body, &candidates, &helpers, &array);
        }
    }
}

impl<'a> Visit<'a> for Analysis<'_, '_, 'a> {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        if let Some(body) = &func.body {
            self.analyze_function(&func.params, body, func.scope_id.get());
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        if let Some(body) = arrow.body.as_function_body() {
            self.analyze_function(&arrow.params, body, arrow.scope_id.get());
        }
        walk::walk_arrow_function_expression(self, arrow);
    }
}

/// Declared once and never assigned.
fn is_fixed(scoping: &Scoping, symbol: SymbolId) -> bool {
    scoping.symbol_redeclarations(symbol).is_empty()
        && !scoping
            .get_resolved_reference_ids(symbol)
            .iter()
            .any(|&id| scoping.get_reference(id).is_write())
}

impl<'a> Analysis<'_, '_, 'a> {
    fn analyze_array(
        &mut self,
        body: &Body<'_, 'a>,
        candidates: &[DecoderCandidate<'_, 'a>],
        helpers: &Helpers<'_, '_, '_>,
        array: &StringArray,
    ) -> Option<()> {
        let scoping = self.scoping;
        let decoders: Vec<Decoder> = candidates
            .iter()
            .filter_map(|candidate| match_decoder(candidate, scoping, array, helpers))
            .take(MAX_DECODERS + 1)
            .collect();
        // A wrapper is only read through a decoder; a variable may also be
        // read directly.
        if decoders.len() > MAX_DECODERS || (decoders.is_empty() && array.declared_at.is_none()) {
            return None;
        }
        let helpers = || decoders.iter().filter_map(|decoder| decoder.helper);
        if !scoping.symbol_redeclarations(array.symbol).is_empty()
            || !decoders.iter().all(|decoder| is_fixed(scoping, decoder.symbol))
            || !helpers().all(|(helper, _)| is_fixed(scoping, helper))
        {
            return None;
        }

        let rotations: Vec<_> = body
            .statements
            .iter()
            .enumerate()
            .filter_map(|(index, stmt)| match stmt {
                Statement::ExpressionStatement(expr) => {
                    match_rotation(expr, scoping, array, &decoders).map(|rotation| (index, &**expr, rotation))
                }
                _ => None,
            })
            .collect();
        let mut array_uses: FxHashSet<ReferenceId> = array.own_refs.iter().copied().collect();
        array_uses.extend(decoders.iter().map(|decoder| decoder.array_ref));
        let mut strings = array.strings.clone();
        let rotation = match rotations.as_slice() {
            [] => None,
            [(index, stmt, rotation)] => {
                let steps = rotation.steps(&strings, &decoders, &mut self.budget.rotation_work)?;
                strings.rotate_left(steps);
                array_uses.insert(rotation.array_ref);
                Some((*index, *stmt, rotation.decoder.map(|decoder| &decoders[decoder])))
            }
            _ => return None,
        };
        // Only an array that a decoder reads or a proven rotation reorders is
        // obfuscation; a plain array of strings stays as written.
        if decoders.is_empty() && rotation.is_none() {
            return None;
        }

        // Statements up to here run no code but the rotation, so variables
        // they declare are set before any decoder call. A rotation must come
        // after the array variable it rotates and the decoder variable it calls.
        let prefix: Vec<&Statement<'_>> = body
            .statements
            .iter()
            .take_while(|stmt| {
                is_inert(stmt, scoping, body.scope) || rotation.is_some_and(|(_, rot, _)| is_statement(stmt, rot))
            })
            .collect();
        let after = |at: Option<usize>, index: usize| at.is_some_and(|at| at > index);
        if let Some((index, _, checksum_decoder)) = rotation
            && (index >= prefix.len()
                || after(array.declared_at, index)
                || checksum_decoder.is_some_and(|decoder| {
                    after(decoder.declared_at, index) || decoder.helper.is_some_and(|(_, at)| after(at, index))
                }))
        {
            return None;
        }
        let declarations = || {
            std::iter::once(array.declared_at)
                .chain(decoders.iter().map(|decoder| decoder.declared_at))
                .chain(helpers().map(|(_, at)| at))
        };
        if declarations().any(|at| at.is_some_and(|at| at >= prefix.len())) {
            return None;
        }
        // An imported module may call into this one before its body runs.
        let needs_order = rotation.is_some() || declarations().any(|at| at.is_some());
        if needs_order
            && body.function.is_none()
            && self
                .program
                .body
                .iter()
                .any(|stmt| matches!(stmt, Statement::ImportDeclaration(_)))
        {
            return None;
        }

        // Each analysis walks the whole body, so the walks share a budget.
        let span = body.function.map_or(self.program.span, |function| function.span);
        self.budget.walk_bytes = self.budget.walk_bytes.checked_sub(span.size() as usize)?;
        // Aliases declared in that prefix hold the decoder wherever they are read.
        let early_declarations = prefix
            .iter()
            .filter_map(|stmt| match stmt {
                Statement::VariableDeclaration(decl) => Some(address(&**decl)),
                _ => None,
            })
            .collect();
        let rotation_stmt = rotation.map(|(_, stmt, _)| address(stmt));
        let mut walker = Walker::new(scoping, &decoders, array, &strings, rotation_stmt, early_declarations);
        walker.inline_budget = self.budget.inline_bytes;
        if let Some(function) = body.function {
            walker.visit_function_body(function);
        } else {
            walker.script = self.program.source_type.is_script();
            walker.visit_program(self.program);
        }
        self.budget.inline_bytes = walker.inline_budget;
        walker.finish(array_uses, &self.shadowed, self.plan);
        Some(())
    }
}

fn is_statement(stmt: &Statement<'_>, expr: &ExpressionStatement<'_>) -> bool {
    matches!(stmt, Statement::ExpressionStatement(candidate) if address(&**candidate) == address(expr))
}

/// A statement of a body that runs no code: a function declaration, or a
/// variable declaration whose initialisers only create a function or an
/// array of string literals, or read a function the body declares.
fn is_inert(stmt: &Statement<'_>, scoping: &Scoping, scope: ScopeId) -> bool {
    match stmt {
        Statement::FunctionDeclaration(_) | Statement::EmptyStatement(_) => true,
        Statement::VariableDeclaration(decl) => {
            matches!(
                decl.kind,
                VariableDeclarationKind::Var | VariableDeclarationKind::Let | VariableDeclarationKind::Const
            ) && decl.declarations.iter().all(|declarator| match &declarator.init {
                None | Some(Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)) => true,
                Some(Expression::ArrayExpression(array)) => array
                    .elements
                    .iter()
                    .all(|element| matches!(element, ArrayExpressionElement::StringLiteral(_))),
                Some(Expression::Identifier(id)) => reference_symbol(scoping, id).is_some_and(|symbol| {
                    scoping.symbol_flags(symbol).is_function()
                        && scoping.symbol_scope_id(symbol) == scope
                        && scoping.symbol_redeclarations(symbol).is_empty()
                }),
                Some(_) => false,
            })
        }
        _ => false,
    }
}

/// The address of a member expression `expr` is, through parentheses.
fn member_address(expr: &Expression<'_>) -> Option<usize> {
    match expr.without_parentheses() {
        Expression::ComputedMemberExpression(member) => Some(address(&**member)),
        Expression::StaticMemberExpression(member) => Some(address(&**member)),
        _ => None,
    }
}

/// A copy of `value` when the inline budget still allows it, charged to
/// `budget`.
fn charge(budget: &mut usize, value: &str) -> Option<String> {
    *budget = budget.checked_sub(value.len())?;
    Some(value.to_owned())
}

/// A binding initialised to a decoder or to another alias, never reassigned.
struct Alias {
    parent: SymbolId,
    /// The index of the decoder it holds.
    decoder: usize,
    /// The number of enclosing functions at its declaration.
    fn_depth: usize,
    /// Declared at the top level before any code but the rotation runs.
    early: bool,
    /// Declared at the top level of a classic script, where other scripts
    /// see it.
    global: bool,
}

struct Walker<'s> {
    scoping: &'s Scoping,
    decoders: &'s [Decoder],
    array: SymbolId,
    /// The array when it is a variable, which code may read directly.
    variable_array: Option<SymbolId>,
    strings: &'s [String],
    /// The decoders, their helpers, and the array.
    guarded: Vec<SymbolId>,
    rotation: Option<usize>,
    in_rotation: bool,
    with_depth: usize,
    /// For each enclosing function, whether it is a hoisted declaration.
    frames: Vec<bool>,
    /// The statement list of the function body or program being entered.
    body_list: Option<usize>,
    /// The next variable declaration when it is a statement of a block-like
    /// list, and whether that list is a function body or the program.
    listed_declaration: Option<(usize, bool)>,
    early_declarations: FxHashSet<usize>,
    aliases: FxHashMap<SymbolId, Alias>,
    alias_order: Vec<SymbolId>,
    /// References of a decoder or an alias inside the rotation.
    rotation_refs: FxHashMap<SymbolId, usize>,
    /// Reads of the array's own data properties, `a[N]` and `a.length`, by
    /// the reference to `a`.
    member_reads: FxHashSet<ReferenceId>,
    /// Number of `a[N]` reads replaced.
    replaced_reads: usize,
    /// Member expressions in a position that may pass the array as `this`
    /// or delete a property: a callee, a tag, or `delete`'s operand.
    non_reads: FxHashSet<usize>,
    /// Replaced expressions by reference, the symbol it resolves to, and the
    /// string read.
    replacements: Vec<(ReferenceId, SymbolId, String)>,
    /// Decoded elements by decoder and index.
    decoded: FxHashMap<(usize, usize), Option<String>>,
    /// Bytes still allowed to be inlined.
    inline_budget: usize,
    /// A classic script, whose top-level bindings are shared with other
    /// scripts and reachable through the global object.
    script: bool,
    /// Enclosing strict-mode functions and class members, which give `this`
    /// a value other than the global object.
    this_depth: usize,
    /// Properties named on the global object (`window.x`, `this.x` at the top level).
    global_members: FxHashSet<String>,
    /// The global object is used other than as `typeof` or by a static
    /// member, or code may reach `Function` through `.constructor`.
    global_object_escapes: bool,
}

impl<'s> Walker<'s> {
    fn new(
        scoping: &'s Scoping,
        decoders: &'s [Decoder],
        array: &StringArray,
        strings: &'s [String],
        rotation: Option<usize>,
        early_declarations: FxHashSet<usize>,
    ) -> Self {
        let mut guarded: Vec<SymbolId> = decoders.iter().map(|decoder| decoder.symbol).collect();
        guarded.extend(
            decoders
                .iter()
                .filter_map(|decoder| decoder.helper.map(|(helper, _)| helper)),
        );
        guarded.push(array.symbol);
        Self {
            scoping,
            decoders,
            array: array.symbol,
            variable_array: array.declared_at.map(|_| array.symbol),
            strings,
            guarded,
            rotation,
            in_rotation: false,
            with_depth: 0,
            frames: Vec::new(),
            body_list: None,
            listed_declaration: None,
            early_declarations,
            aliases: FxHashMap::default(),
            alias_order: Vec::new(),
            rotation_refs: FxHashMap::default(),
            member_reads: FxHashSet::default(),
            replaced_reads: 0,
            non_reads: FxHashSet::default(),
            replacements: Vec::new(),
            decoded: FxHashMap::default(),
            inline_budget: 0,
            script: false,
            this_depth: 0,
            global_members: FxHashSet::default(),
            global_object_escapes: false,
        }
    }

    /// The index of the decoder `symbol` is or holds.
    fn decoder_of(&self, symbol: SymbolId) -> Option<usize> {
        self.decoders
            .iter()
            .position(|decoder| decoder.symbol == symbol)
            .or_else(|| self.aliases.get(&symbol).map(|alias| alias.decoder))
    }

    /// Whether a reference here runs after the rotation and outside any
    /// `with` body, which could redirect it.
    const fn in_plain_code(&self) -> bool {
        !self.in_rotation && self.with_depth == 0
    }

    /// Whether a reference to `symbol` here runs only after the decoder or
    /// alias holds the decoder: in plain code, and for an alias, not inside a
    /// function hoisted above its declaration. The walk sees an alias only
    /// after its declarator, so a reference it sees is later in the source.
    fn can_use(&self, symbol: SymbolId) -> bool {
        if !self.in_plain_code() {
            return false;
        }
        let Some(alias) = self.aliases.get(&symbol) else {
            return self.decoders.iter().any(|decoder| decoder.symbol == symbol);
        };
        alias.early
            || self
                .frames
                .get(alias.fn_depth..)
                .is_some_and(|inner| inner.iter().all(|hoisted| !hoisted))
    }

    fn register_alias(&mut self, declarator: &VariableDeclarator<'_>, early: bool, global: bool) {
        let BindingPattern::BindingIdentifier(binding) = &declarator.id else {
            return;
        };
        let Some(Expression::Identifier(init)) = &declarator.init else {
            return;
        };
        let (Some(symbol), Some(parent)) = (binding.symbol_id.get(), reference_symbol(self.scoping, init)) else {
            return;
        };
        if let Some(decoder) = self.decoder_of(parent)
            && self.can_use(parent)
            && is_fixed(self.scoping, symbol)
        {
            self.aliases.insert(
                symbol,
                Alias {
                    parent,
                    decoder,
                    fn_depth: self.frames.len(),
                    early,
                    global,
                },
            );
            self.alias_order.push(symbol);
        }
    }

    fn try_replace_call(&mut self, call: &CallExpression<'_>) {
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let (Some(reference), Some(symbol)) = (callee.reference_id.get(), reference_symbol(self.scoping, callee))
        else {
            return;
        };
        let Some(decoder) = self.decoder_of(symbol) else {
            return;
        };
        let decoders = self.decoders;
        let Some(arg) = index_literal(call.arguments.as_slice(), decoders[decoder].coerces_strings) else {
            return;
        };
        if call.optional || !self.can_use(symbol) || self.inline_budget == 0 {
            return;
        }
        let Some(index) = array_index(decoders[decoder].index(arg), self.strings.len()) else {
            return;
        };
        let raw = &self.strings[index];
        let value = match &decoders[decoder].decoding {
            Decoding::Plain => charge(&mut self.inline_budget, raw),
            decoding => self
                .decoded
                .entry((decoder, index))
                .or_insert_with(|| decoding.apply(raw).map(String::from))
                .as_deref()
                .and_then(|value| charge(&mut self.inline_budget, value)),
        };
        if let Some(value) = value {
            self.replacements.push((reference, symbol, value));
        }
    }

    /// Records a read of the array's own data property, `a.length` or `a[N]`
    /// for an index `N` in range, and plans to replace `a[N]`. Any other
    /// property may be an accessor that receives the array as `this`.
    fn record_member_read(&mut self, expr: &Expression<'_>) {
        let Some(array) = self.variable_array else {
            return;
        };
        let (object, index) = match expr {
            Expression::ComputedMemberExpression(member) if !member.optional => match &member.expression {
                Expression::NumericLiteral(index) => match array_index(index.value, self.strings.len()) {
                    Some(index) => (&member.object, Some(index)),
                    None => return,
                },
                _ => return,
            },
            Expression::StaticMemberExpression(member) if !member.optional && member.property.name == "length" => {
                (&member.object, None)
            }
            _ => return,
        };
        let Expression::Identifier(object) = object else {
            return;
        };
        let Some(reference) = object.reference_id.get() else {
            return;
        };
        if reference_symbol(self.scoping, object) != Some(array)
            || member_address(expr).is_some_and(|member| self.non_reads.contains(&member))
        {
            return;
        }
        self.member_reads.insert(reference);
        if self.in_plain_code()
            && let Some(index) = index
            && let Some(value) = charge(&mut self.inline_budget, &self.strings[index])
        {
            self.replacements.push((reference, array, value));
            self.replaced_reads += 1;
        }
    }

    /// Marks `expr` as a callee, tag or `delete` operand when it is a
    /// member expression.
    fn mark_non_read(&mut self, expr: &Expression<'_>) {
        if let Some(member) = member_address(expr) {
            self.non_reads.insert(member);
        }
    }

    /// Whether code can overwrite a decoder, a helper, the array or a
    /// top-level alias through the global object.
    fn global_object_may_overwrite(&self) -> bool {
        if !self.script {
            return false;
        }
        let global_aliases = self
            .aliases
            .iter()
            .filter(|(_, alias)| alias.global)
            .map(|(&symbol, _)| symbol);
        self.global_object_escapes
            || self
                .guarded
                .iter()
                .copied()
                .chain(global_aliases)
                .any(|symbol| self.global_members.contains(self.scoping.symbol_name(symbol)))
    }

    fn is_global_object(&self, expr: &Expression<'_>) -> bool {
        match expr.without_parentheses() {
            Expression::ThisExpression(_) => self.this_depth == 0,
            Expression::Identifier(id) => self.is_global_object_name(id),
            _ => false,
        }
    }

    fn is_global_object_name(&self, id: &IdentifierReference<'_>) -> bool {
        GLOBAL_OBJECT_NAMES.contains(&id.name.as_str())
            && id
                .reference_id
                .get()
                .is_some_and(|reference| self.scoping.get_reference(reference).symbol_id().is_none())
    }

    fn finish(self, mut array_uses: FxHashSet<ReferenceId>, shadowed: &FxHashSet<SymbolId>, plan: &mut Plan) {
        // Nothing but the decoders, the rotation and property reads may reach
        // the array.
        array_uses.extend(self.member_reads.iter().copied());
        let array_refs = self.scoping.get_resolved_reference_ids(self.array);
        if array_refs.len() != array_uses.len() || !array_refs.iter().all(|id| array_uses.contains(id)) {
            return;
        }
        if self.guarded.iter().any(|symbol| shadowed.contains(symbol)) || self.global_object_may_overwrite() {
            return;
        }
        // A block function can overwrite an alias, and with it every alias
        // of that alias.
        let mut overwritable = FxHashSet::default();
        for &symbol in &self.alias_order {
            if shadowed.contains(&symbol) || overwritable.contains(&self.aliases[&symbol].parent) {
                overwritable.insert(symbol);
            }
        }

        let mut accounted = self.rotation_refs;
        let replacements: Vec<_> = self
            .replacements
            .into_iter()
            .filter(|(_, symbol, _)| !overwritable.contains(symbol))
            .collect();
        for (_, symbol, _) in &replacements {
            *accounted.entry(*symbol).or_default() += 1;
        }
        let total = |symbol| self.scoping.get_resolved_reference_ids(symbol).len();
        for &alias in self.alias_order.iter().rev() {
            let info = &self.aliases[&alias];
            // Other scripts may read a top-level alias, so it stays.
            if !info.global
                && !overwritable.contains(&alias)
                && accounted.get(&alias).copied().unwrap_or(0) == total(alias)
            {
                plan.dead_aliases.insert(alias);
                *accounted.entry(info.parent).or_default() += 1;
            }
        }
        // The rotation only reorders the array, so it can go once no decoder
        // reference or property read is left to observe the order.
        // Other scripts may call a global decoder, so in a script it stays.
        let all_replaced = |symbol| accounted.get(&symbol).copied().unwrap_or(0) == total(symbol);
        if let Some(rotation) = self.rotation
            && !self.script
            && self.decoders.iter().all(|decoder| all_replaced(decoder.symbol))
            && self.replaced_reads == self.member_reads.len()
        {
            plan.rotations.push(rotation);
        }
        plan.replacements
            .extend(replacements.into_iter().map(|(reference, _, value)| (reference, value)));
    }
}

impl<'a> Visit<'a> for Walker<'_> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.body_list = Some(address(&program.body));
        walk::walk_program(self, program);
    }

    fn visit_function_body(&mut self, body: &FunctionBody<'a>) {
        self.body_list = Some(address(&body.statements));
        walk::walk_function_body(self, body);
    }

    fn visit_statements(&mut self, stmts: &oxc_allocator::Vec<'a, Statement<'a>>) {
        let is_body = self.body_list.take() == Some(address(stmts));
        for stmt in stmts {
            if let Statement::VariableDeclaration(decl) = stmt {
                self.listed_declaration = Some((address(&**decl), is_body));
            }
            self.visit_statement(stmt);
        }
    }

    fn visit_switch_case(&mut self, case: &SwitchCase<'a>) {
        // A case can be entered past a declaration of an earlier case, so its
        // declarations are not listed.
        if let Some(test) = &case.test {
            self.visit_expression(test);
        }
        for stmt in &case.consequent {
            self.visit_statement(stmt);
        }
    }

    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        let (listed, in_body) = match self.listed_declaration.take() {
            Some((listed, in_body)) if listed == address(decl) => match decl.kind {
                VariableDeclarationKind::Const | VariableDeclarationKind::Let => (true, in_body),
                // A `var` in a block may be skipped while later code still sees it.
                VariableDeclarationKind::Var => (in_body, in_body),
                _ => (false, false),
            },
            _ => (false, false),
        };
        let early = self.early_declarations.contains(&address(decl));
        let global = self.script && in_body && self.frames.is_empty();
        for declarator in &decl.declarations {
            self.visit_variable_declarator(declarator);
            if listed {
                self.register_alias(declarator, early, global);
            }
        }
    }

    fn visit_expression_statement(&mut self, stmt: &ExpressionStatement<'a>) {
        let is_rotation = self.rotation == Some(address(stmt));
        self.in_rotation |= is_rotation;
        walk::walk_expression_statement(self, stmt);
        if is_rotation {
            self.in_rotation = false;
        }
    }

    fn visit_expression(&mut self, expr: &Expression<'a>) {
        self.record_member_read(expr);
        walk::walk_expression(self, expr);
    }

    fn visit_chain_element(&mut self, element: &ChainElement<'a>) {
        // A call that is itself a chain element cannot become a string literal.
        if let ChainElement::CallExpression(call) = element {
            self.mark_non_read(&call.callee);
            walk::walk_call_expression(self, call);
        } else {
            walk::walk_chain_element(self, element);
        }
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        self.mark_non_read(&call.callee);
        self.try_replace_call(call);
        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, new: &NewExpression<'a>) {
        self.mark_non_read(&new.callee);
        walk::walk_new_expression(self, new);
    }

    fn visit_tagged_template_expression(&mut self, tagged: &TaggedTemplateExpression<'a>) {
        self.mark_non_read(&tagged.tag);
        walk::walk_tagged_template_expression(self, tagged);
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        // `f.constructor` is `Function`, which runs code in the global scope.
        self.global_object_escapes |= self.script && member.property.name == "constructor";
        if self.script && self.is_global_object(&member.object) {
            self.global_members.insert(member.property.name.to_string());
        } else {
            walk::walk_static_member_expression(self, member);
        }
    }

    fn visit_computed_member_expression(&mut self, member: &ComputedMemberExpression<'a>) {
        self.global_object_escapes |=
            self.script && matches!(&member.expression, Expression::StringLiteral(key) if key.value == "constructor");
        walk::walk_computed_member_expression(self, member);
    }

    fn visit_unary_expression(&mut self, unary: &UnaryExpression<'a>) {
        if unary.operator == UnaryOperator::Delete {
            self.mark_non_read(&unary.argument);
        }
        let is_typeof_global = unary.operator == UnaryOperator::Typeof
            && matches!(&unary.argument, Expression::Identifier(id) if self.is_global_object_name(id));
        if !is_typeof_global {
            walk::walk_unary_expression(self, unary);
        }
    }

    fn visit_this_expression(&mut self, _this: &ThisExpression) {
        if self.script && self.this_depth == 0 {
            self.global_object_escapes = true;
        }
    }

    fn visit_property_definition(&mut self, property: &PropertyDefinition<'a>) {
        self.visit_property_key(&property.key);
        self.this_depth += 1;
        if let Some(value) = &property.value {
            self.visit_expression(value);
        }
        self.this_depth -= 1;
    }

    fn visit_accessor_property(&mut self, property: &AccessorProperty<'a>) {
        self.visit_property_key(&property.key);
        self.this_depth += 1;
        if let Some(value) = &property.value {
            self.visit_expression(value);
        }
        self.this_depth -= 1;
    }

    fn visit_static_block(&mut self, block: &StaticBlock<'a>) {
        self.this_depth += 1;
        walk::walk_static_block(self, block);
        self.this_depth -= 1;
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if self.script && self.is_global_object_name(ident) {
            self.global_object_escapes = true;
        }
        if self.in_rotation
            && let Some(symbol) = reference_symbol(self.scoping, ident)
            && self.decoder_of(symbol).is_some()
        {
            *self.rotation_refs.entry(symbol).or_default() += 1;
        }
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        // A sloppy-mode function called without a receiver gets the global
        // object as `this`.
        let strict = func
            .scope_id
            .get()
            .is_some_and(|scope| self.scoping.scope_flags(scope).is_strict_mode());
        self.frames.push(func.is_declaration());
        self.this_depth += usize::from(strict);
        walk::walk_function(self, func, flags);
        self.this_depth -= usize::from(strict);
        self.frames.pop();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.frames.push(false);
        walk::walk_arrow_function_expression(self, arrow);
        self.frames.pop();
    }

    fn visit_with_statement(&mut self, stmt: &WithStatement<'a>) {
        self.with_depth += 1;
        walk::walk_with_statement(self, stmt);
        self.with_depth -= 1;
    }
}
