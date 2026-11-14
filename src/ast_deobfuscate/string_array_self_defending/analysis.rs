//! Finds the string array, its decoder, and the rotation IIFE, proves which
//! decoder calls read a known string, and plans the rewrite.

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, ArrowFunctionExpression, BindingPattern, CallExpression, ChainElement,
    Expression, ExpressionStatement, Function, FunctionBody, IdentifierReference, Program, Statement, SwitchCase,
    VariableDeclaration, VariableDeclarationKind, VariableDeclarator, WithStatement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Scoping;
use oxc_syntax::reference::ReferenceId;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::checksum::{array_index, index_literal};
use super::shape::{
    Decoder, DecoderCandidate, RotationKind, StringArray, decoder_candidate, match_array_variable, match_decoder,
    match_rotation, match_wrapper, reference_symbol,
};

/// Upper bound on the string arrays analysed per program; each costs one walk.
const MAX_ARRAYS: usize = 8;
/// Inlined bytes allowed per source byte, so that inlining one long string
/// at many call sites cannot exhaust memory.
const INLINE_BYTES_PER_SOURCE_BYTE: usize = 4;
/// Inlined bytes always allowed, however short the source.
const MIN_INLINE_BYTES: usize = 1 << 20;

#[derive(Default)]
pub(super) struct Plan {
    /// Decoder call sites, by their callee's reference, and the string each reads.
    pub(super) replacements: FxHashMap<ReferenceId, String>,
    /// Addresses of the rotation IIFE statements to remove.
    pub(super) rotations: Vec<usize>,
    /// Decoder aliases whose every reference is replaced.
    pub(super) dead_aliases: FxHashSet<SymbolId>,
}

pub(super) fn address<T>(node: &T) -> usize {
    std::ptr::from_ref(node).addr()
}

pub(super) fn analyze(program: &Program<'_>, scoping: &Scoping) -> Plan {
    let mut plan = Plan::default();
    if scoping.root_scope_flags().contains_direct_eval() {
        return plan;
    }
    let arrays = program.body.iter().enumerate().filter_map(|(index, stmt)| match stmt {
        Statement::FunctionDeclaration(func) => match_wrapper(func, scoping),
        Statement::VariableDeclaration(decl) => match_array_variable(decl, index),
        _ => None,
    });
    let decoders: Vec<DecoderCandidate<'_, '_>> = program
        .body
        .iter()
        .enumerate()
        .filter_map(|(index, stmt)| decoder_candidate(stmt, index))
        .collect();
    let mut inline_budget = program
        .source_text
        .len()
        .saturating_mul(INLINE_BYTES_PER_SOURCE_BYTE)
        .max(MIN_INLINE_BYTES);
    for array in arrays.take(MAX_ARRAYS) {
        analyze_array(program, scoping, &decoders, &array, &mut inline_budget, &mut plan);
    }
    plan
}

fn analyze_array(
    program: &Program<'_>,
    scoping: &Scoping,
    candidates: &[DecoderCandidate<'_, '_>],
    array: &StringArray,
    inline_budget: &mut usize,
    plan: &mut Plan,
) -> Option<()> {
    let decoders: Vec<Decoder> = candidates
        .iter()
        .filter_map(|candidate| match_decoder(candidate, scoping, array))
        .collect();
    let [decoder] = decoders.as_slice() else {
        return None;
    };
    let decoder_refs = scoping.get_resolved_reference_ids(decoder.symbol);
    if !scoping.symbol_redeclarations(array.symbol).is_empty()
        || !scoping.symbol_redeclarations(decoder.symbol).is_empty()
        || decoder_refs.iter().any(|&id| scoping.get_reference(id).is_write())
    {
        return None;
    }

    let rotations: Vec<_> = program
        .body
        .iter()
        .enumerate()
        .filter_map(|(index, stmt)| match stmt {
            Statement::ExpressionStatement(expr) => {
                match_rotation(expr, scoping, array, decoder).map(|rotation| (index, &**expr, rotation))
            }
            _ => None,
        })
        .collect();
    let mut array_uses = array.own_refs.clone();
    array_uses.push(decoder.array_ref);
    let mut strings = array.strings.clone();
    let rotation = match rotations.as_slice() {
        [] => None,
        [(index, stmt, rotation)] => {
            let steps = rotation.steps(&strings, decoder.offset)?;
            strings.rotate_left(steps);
            array_uses.push(rotation.array_ref);
            Some((*index, *stmt, matches!(rotation.kind, RotationKind::Checksum { .. })))
        }
        _ => return None,
    };
    // Nothing but the decoder and the rotation may reach the array.
    let array_refs = scoping.get_resolved_reference_ids(array.symbol);
    if array_refs.len() != array_uses.len() || !array_refs.iter().all(|id| array_uses.contains(id)) {
        return None;
    }

    // Statements up to here run no code but the rotation, so variables they
    // declare are set before any decoder call. A rotation must come after
    // the array variable it rotates and the decoder variable it calls.
    let prefix: Vec<&Statement<'_>> = program
        .body
        .iter()
        .take_while(|stmt| is_inert(stmt, scoping) || rotation.is_some_and(|(_, rot, _)| is_statement(stmt, rot)))
        .collect();
    if let Some((index, _, calls_decoder)) = rotation
        && (index >= prefix.len()
            || array.declared_at.is_some_and(|at| at > index)
            || (calls_decoder && decoder.declared_at.is_some_and(|at| at > index)))
    {
        return None;
    }
    let declared_late = |at: Option<usize>| at.is_some_and(|at| at >= prefix.len());
    if declared_late(array.declared_at) || declared_late(decoder.declared_at) {
        return None;
    }
    // An imported module may call into this one before its body runs.
    let needs_order = rotation.is_some() || array.declared_at.is_some() || decoder.declared_at.is_some();
    if needs_order
        && program
            .body
            .iter()
            .any(|stmt| matches!(stmt, Statement::ImportDeclaration(_)))
    {
        return None;
    }

    // Aliases declared in that prefix hold the decoder wherever they are read.
    let early_declarations = prefix
        .iter()
        .filter_map(|stmt| match stmt {
            Statement::VariableDeclaration(decl) => Some(address(&**decl)),
            _ => None,
        })
        .collect();
    let rotation_stmt = rotation.map(|(_, stmt, _)| address(stmt));
    let mut walker = Walker::new(
        scoping,
        decoder,
        array.symbol,
        &strings,
        rotation_stmt,
        early_declarations,
    );
    walker.inline_budget = *inline_budget;
    walker.visit_program(program);
    *inline_budget = walker.inline_budget;
    walker.finish(rotation_stmt, plan);
    Some(())
}

fn is_statement(stmt: &Statement<'_>, expr: &ExpressionStatement<'_>) -> bool {
    matches!(stmt, Statement::ExpressionStatement(candidate) if address(&**candidate) == address(expr))
}

/// A top-level statement that runs no code: a function declaration, or a
/// variable declaration whose initialisers only create a function or an
/// array of string literals, or read a top-level function.
fn is_inert(stmt: &Statement<'_>, scoping: &Scoping) -> bool {
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
                        && scoping.symbol_scope_id(symbol) == scoping.root_scope_id()
                        && scoping.symbol_redeclarations(symbol).is_empty()
                }),
                Some(_) => false,
            })
        }
        _ => false,
    }
}

/// A binding initialised to the decoder or to another alias, never reassigned.
struct Alias {
    parent: SymbolId,
    /// The number of enclosing functions at its declaration.
    fn_depth: usize,
    /// Declared at the top level before any code but the rotation runs.
    early: bool,
    /// A `var`, which a sloppy-mode block function of the same name could
    /// overwrite (Annex B).
    is_var: bool,
}

struct Walker<'s> {
    scoping: &'s Scoping,
    decoder: SymbolId,
    offset: f64,
    coerces_strings: bool,
    strings: &'s [String],
    /// The decoder and the array.
    guarded: [SymbolId; 2],
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
    /// The next function declaration when it is a statement of a function
    /// body or the program.
    listed_function: Option<usize>,
    /// Names of function declarations nested in blocks or statements.
    block_function_names: FxHashSet<String>,
    early_declarations: FxHashSet<usize>,
    aliases: FxHashMap<SymbolId, Alias>,
    alias_order: Vec<SymbolId>,
    /// References of the decoder or an alias inside the rotation.
    rotation_refs: FxHashMap<SymbolId, usize>,
    /// Decoder calls by callee reference, the symbol called, and the string read.
    replacements: Vec<(ReferenceId, SymbolId, String)>,
    /// Bytes still allowed to be inlined.
    inline_budget: usize,
}

impl<'s> Walker<'s> {
    fn new(
        scoping: &'s Scoping,
        decoder: &Decoder,
        array: SymbolId,
        strings: &'s [String],
        rotation: Option<usize>,
        early_declarations: FxHashSet<usize>,
    ) -> Self {
        Self {
            scoping,
            decoder: decoder.symbol,
            offset: decoder.offset,
            coerces_strings: decoder.coerces_strings,
            strings,
            guarded: [decoder.symbol, array],
            rotation,
            in_rotation: false,
            with_depth: 0,
            frames: Vec::new(),
            body_list: None,
            listed_declaration: None,
            listed_function: None,
            block_function_names: FxHashSet::default(),
            early_declarations,
            aliases: FxHashMap::default(),
            alias_order: Vec::new(),
            rotation_refs: FxHashMap::default(),
            replacements: Vec::new(),
            inline_budget: 0,
        }
    }

    fn is_tracked(&self, symbol: SymbolId) -> bool {
        symbol == self.decoder || self.aliases.contains_key(&symbol)
    }

    /// Whether a reference to `symbol` here runs only after the decoder or
    /// alias holds the decoder: not inside the rotation or a `with` body, and
    /// for an alias, not inside a function hoisted above its declaration.
    /// The walk sees an alias only after its declarator, so a reference it
    /// sees is later in the source.
    fn can_use(&self, symbol: SymbolId) -> bool {
        if self.in_rotation || self.with_depth > 0 {
            return false;
        }
        if symbol == self.decoder {
            return true;
        }
        self.aliases.get(&symbol).is_some_and(|alias| {
            alias.early
                || self
                    .frames
                    .get(alias.fn_depth..)
                    .is_some_and(|inner| inner.iter().all(|hoisted| !hoisted))
        })
    }

    fn register_alias(&mut self, declarator: &VariableDeclarator<'_>, early: bool, is_var: bool) {
        let BindingPattern::BindingIdentifier(binding) = &declarator.id else {
            return;
        };
        let Some(Expression::Identifier(init)) = &declarator.init else {
            return;
        };
        let (Some(symbol), Some(parent)) = (binding.symbol_id.get(), reference_symbol(self.scoping, init)) else {
            return;
        };
        let never_reassigned = self.scoping.symbol_redeclarations(symbol).is_empty()
            && !self
                .scoping
                .get_resolved_reference_ids(symbol)
                .iter()
                .any(|&id| self.scoping.get_reference(id).is_write());
        if self.is_tracked(parent) && self.can_use(parent) && never_reassigned {
            self.aliases.insert(
                symbol,
                Alias {
                    parent,
                    fn_depth: self.frames.len(),
                    early,
                    is_var,
                },
            );
            self.alias_order.push(symbol);
        }
    }

    fn try_replace(&mut self, call: &CallExpression<'_>) {
        let Expression::Identifier(callee) = &call.callee else {
            return;
        };
        let Some(arg) = index_literal(call.arguments.as_slice(), self.coerces_strings) else {
            return;
        };
        let (Some(reference), Some(symbol)) = (callee.reference_id.get(), reference_symbol(self.scoping, callee))
        else {
            return;
        };
        if call.optional || !self.is_tracked(symbol) || !self.can_use(symbol) {
            return;
        }
        if let Some(index) = array_index(arg + self.offset, self.strings.len())
            && let Some(budget) = self.inline_budget.checked_sub(self.strings[index].len())
        {
            self.inline_budget = budget;
            self.replacements.push((reference, symbol, self.strings[index].clone()));
        }
    }

    fn finish(self, rotation: Option<usize>, plan: &mut Plan) {
        let names = &self.block_function_names;
        if self
            .guarded
            .iter()
            .any(|&symbol| names.contains(self.scoping.symbol_name(symbol)))
        {
            return;
        }
        // A block function can overwrite a `var` alias, and with it every
        // alias of that alias.
        let mut overwritable = FxHashSet::default();
        for &symbol in &self.alias_order {
            let alias = &self.aliases[&symbol];
            if (alias.is_var && names.contains(self.scoping.symbol_name(symbol)))
                || overwritable.contains(&alias.parent)
            {
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
            if !overwritable.contains(&alias) && accounted.get(&alias).copied().unwrap_or(0) == total(alias) {
                plan.dead_aliases.insert(alias);
                *accounted.entry(self.aliases[&alias].parent).or_default() += 1;
            }
        }
        // The rotation only reorders the array, so it can go once no decoder
        // reference is left to observe the order.
        if let Some(rotation) = rotation
            && accounted.get(&self.decoder).copied().unwrap_or(0) == total(self.decoder)
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
            } else if let Statement::FunctionDeclaration(func) = stmt
                && is_body
            {
                self.listed_function = Some(address(&**func));
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
        let listed = match self.listed_declaration.take() {
            Some((listed, in_body)) if listed == address(decl) => match decl.kind {
                VariableDeclarationKind::Const | VariableDeclarationKind::Let => true,
                // A `var` in a block may be skipped while later code still sees it.
                VariableDeclarationKind::Var => in_body,
                _ => false,
            },
            _ => false,
        };
        let early = self.early_declarations.contains(&address(decl));
        let is_var = decl.kind == VariableDeclarationKind::Var;
        for declarator in &decl.declarations {
            self.visit_variable_declarator(declarator);
            if listed {
                self.register_alias(declarator, early, is_var);
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

    fn visit_chain_element(&mut self, element: &ChainElement<'a>) {
        // A call that is itself a chain element cannot become a string literal.
        if let ChainElement::CallExpression(call) = element {
            walk::walk_call_expression(self, call);
        } else {
            walk::walk_chain_element(self, element);
        }
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        self.try_replace(call);
        walk::walk_call_expression(self, call);
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if self.in_rotation
            && let Some(symbol) = reference_symbol(self.scoping, ident)
            && self.is_tracked(symbol)
        {
            *self.rotation_refs.entry(symbol).or_default() += 1;
        }
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        let is_listed = self.listed_function.take() == Some(address(func));
        if func.is_declaration()
            && !is_listed
            && let Some(id) = &func.id
        {
            self.block_function_names.insert(id.name.to_string());
        }
        self.frames.push(func.is_declaration());
        walk::walk_function(self, func, flags);
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
