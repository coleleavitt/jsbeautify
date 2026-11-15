//! String-array decoders that transform the element they read: the global
//! `atob`, or a helper function whose body XORs the element with a key or
//! decrypts it with RC4. A helper is recognised by its body, matched through
//! oxc symbols, never by its name; the key is a literal at the call.

use std::borrow::Cow;
use std::cell::RefCell;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use oxc_ast::ast::{
    Argument, AssignmentOperator, AssignmentTarget, BinaryExpression, BinaryOperator, Expression, ForStatementInit,
    Function, IdentifierReference, SimpleAssignmentTarget, Statement, UpdateOperator, VariableDeclaration,
    VariableDeclarationKind,
};
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;

use super::shape::{DecoderCandidate, binding_symbol, is_identifier_of, is_method_call, is_plain, reference_symbol};
use crate::ast_deobfuscate::webpack::shapes::simple_params;

/// What a decoder does to the element it reads before returning it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decoding {
    /// Returns the element.
    Plain,
    /// `atob(element)`, the global `atob`.
    Base64,
    /// A helper that XORs each UTF-16 code unit with the key string's,
    /// cycling through the key.
    XorString(Vec<u16>),
    /// A helper that XORs each UTF-16 code unit with a number key: the low
    /// 16 bits of its `ToInt32`.
    XorNumber(u16),
    /// A helper running RC4 with the key string's code units, on the
    /// element's code units or, after `atob`, on its bytes.
    Rc4 { key: Vec<u16>, base64: bool },
}

impl Decoding {
    /// The string the decoder returns for `value`, or `None` when it would
    /// throw or return a string with a lone surrogate.
    pub(super) fn apply<'s>(&self, value: &'s str) -> Option<Cow<'s, str>> {
        let units: Vec<u16> = match self {
            Self::Plain => return Some(Cow::Borrowed(value)),
            Self::Base64 => return Some(Cow::Owned(atob(value)?.into_iter().map(char::from).collect())),
            Self::XorString(key) if key.is_empty() => return Some(Cow::Borrowed(value)),
            Self::XorString(key) => value
                .encode_utf16()
                .zip(key.iter().cycle())
                .map(|(unit, key)| unit ^ key)
                .collect(),
            Self::XorNumber(key) => value.encode_utf16().map(|unit| unit ^ key).collect(),
            Self::Rc4 { key, base64 } => {
                let input: Vec<u16> = if *base64 {
                    atob(value)?.into_iter().map(u16::from).collect()
                } else {
                    value.encode_utf16().collect()
                };
                rc4(&input, key)?
            }
        };
        String::from_utf16(&units).ok().map(Cow::Owned)
    }
}

/// `atob`'s bytes for canonical, padded base64; `None` for any other input,
/// which `atob` may reject or read more leniently.
fn atob(value: &str) -> Option<Vec<u8>> {
    BASE64_STANDARD.decode(value.as_bytes()).ok()
}

/// RC4 over code units, as the matched helper computes it: the key schedule
/// adds whole key code units, and the keystream XORs each input unit.
/// `None` for an empty key, which makes the helper read `NaN` indexes.
fn rc4(input: &[u16], key: &[u16]) -> Option<Vec<u16>> {
    if key.is_empty() {
        return None;
    }
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + usize::from(s[i]) + usize::from(key[i % key.len()])) % 256;
        s.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    Some(
        input
            .iter()
            .map(|&unit| {
                i = (i + 1) % 256;
                j = (j + usize::from(s[i])) % 256;
                s.swap(i, j);
                unit ^ u16::from(s[(usize::from(s[i]) + usize::from(s[j])) % 256])
            })
            .collect(),
    )
}

/// What a decoder returns: its read of the array, through a decoding.
pub(super) struct DecodedRead<'e, 'a> {
    pub(super) decoding: Decoding,
    /// The helper the decoding calls, and the program-body index of its
    /// declaration when it is a variable.
    pub(super) helper: Option<(SymbolId, Option<usize>)>,
    pub(super) read: &'e Expression<'a>,
}

/// The functions of a body that a decoder may call as its helper, each
/// matched at most once.
pub(super) struct Helpers<'c, 'p, 'a> {
    candidates: FxHashMap<SymbolId, &'c DecoderCandidate<'p, 'a>>,
    kinds: RefCell<FxHashMap<SymbolId, Option<Helper>>>,
}

impl<'c, 'p, 'a> Helpers<'c, 'p, 'a> {
    pub(super) fn new(candidates: &'c [DecoderCandidate<'p, 'a>]) -> Self {
        Self {
            candidates: candidates
                .iter()
                .map(|candidate| (candidate.symbol, candidate))
                .collect(),
            kinds: RefCell::default(),
        }
    }

    /// The kind of the helper `symbol` names, and the program-body index of
    /// its declaration when it is a variable.
    fn get(&self, symbol: SymbolId, scoping: &Scoping) -> Option<(Helper, Option<usize>)> {
        let candidate = self.candidates.get(&symbol)?;
        let kind = *self
            .kinds
            .borrow_mut()
            .entry(symbol)
            .or_insert_with(|| match_helper(candidate.func, scoping));
        Some((kind?, candidate.declared_at))
    }
}

/// A decoder's returned expression: the element read itself, `atob(read)`,
/// or `helper(read, <key literal>)` with `helper` one of `helpers`.
pub(super) fn match_decoding<'e, 'a>(
    expr: &'e Expression<'a>,
    scoping: &Scoping,
    helpers: &Helpers<'_, '_, '_>,
) -> Option<DecodedRead<'e, 'a>> {
    let Expression::CallExpression(call) = expr else {
        return Some(DecodedRead {
            decoding: Decoding::Plain,
            helper: None,
            read: expr,
        });
    };
    let Expression::Identifier(callee) = &call.callee else {
        return None;
    };
    if call.optional {
        return None;
    }
    match call.arguments.as_slice() {
        [read] if is_global(scoping, callee, "atob") => Some(DecodedRead {
            decoding: Decoding::Base64,
            helper: None,
            read: read.as_expression()?,
        }),
        [read, key] => {
            let helper = reference_symbol(scoping, callee)?;
            let (kind, declared_at) = helpers.get(helper, scoping)?;
            let decoding = match (kind, key) {
                (Helper::Xor(KeyForm::String), Argument::StringLiteral(key)) => {
                    Decoding::XorString(key.value.encode_utf16().collect())
                }
                (Helper::Xor(KeyForm::Number), Argument::NumericLiteral(key)) => {
                    Decoding::XorNumber(low_16_bits(key.value))
                }
                (Helper::Rc4 { base64 }, Argument::StringLiteral(key)) => Decoding::Rc4 {
                    key: key.value.encode_utf16().collect(),
                    base64,
                },
                _ => return None,
            };
            Some(DecodedRead {
                decoding,
                helper: Some((helper, declared_at)),
                read: read.as_expression()?,
            })
        }
        _ => None,
    }
}

/// The low 16 bits of JavaScript's `ToInt32(value)`.
fn low_16_bits(value: f64) -> u16 {
    if !value.is_finite() {
        return 0;
    }
    // `%` on floats is exact, so this is `value` modulo 2^16 for any integer.
    let low = (value.trunc() % 65_536.0 + 65_536.0) % 65_536.0;
    low as u16
}

/// A bare `name` that reads the global: no binding resolves it, and no code
/// in the program assigns the global by that name.
pub(super) fn is_global(scoping: &Scoping, ident: &IdentifierReference<'_>, name: &str) -> bool {
    ident.name == name
        && ident
            .reference_id
            .get()
            .is_some_and(|reference| scoping.get_reference(reference).symbol_id().is_none())
        && scoping.root_unresolved_references().get(name).is_none_or(|references| {
            references
                .iter()
                .all(|&reference| !scoping.get_reference(reference).is_write())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyForm {
    /// `k.charCodeAt(i % k.length)`
    String,
    /// `k`
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Helper {
    Xor(KeyForm),
    Rc4 { base64: bool },
}

/// The kind of a two-parameter helper `(value, key)` whose whole body is
/// one of the decoder loops below.
fn match_helper(func: &Function<'_>, scoping: &Scoping) -> Option<Helper> {
    let params = simple_params(&func.params)?;
    let [value, key] = params.as_slice() else {
        return None;
    };
    if !is_plain(func) || value == key {
        return None;
    }
    let body = func.body.as_ref()?.statements.as_slice();
    let m = Matcher { scoping };
    m.match_xor(body, *value, *key)
        .map(Helper::Xor)
        .or_else(|| m.match_rc4(body, *value, *key).map(|base64| Helper::Rc4 { base64 }))
}

/// Local bindings a helper's leading declarations introduce, by role.
#[derive(Default)]
struct Locals {
    /// `name = ''`
    result: Option<SymbolId>,
    /// `name = []`
    table: Option<SymbolId>,
    /// `name = 0`
    zero: Option<SymbolId>,
    /// Declared without an initialiser.
    uninit: Vec<SymbolId>,
}

struct Matcher<'s> {
    scoping: &'s Scoping,
}

/// The bindings of an RC4 helper that every loop uses.
#[derive(Clone, Copy)]
struct Rc4Vars {
    /// The parameter holding the encrypted string.
    value: SymbolId,
    key: SymbolId,
    /// The permutation, `s = []`.
    table: SymbolId,
    /// `j = 0`
    j: SymbolId,
    /// The decrypted string, `r = ''`.
    result: SymbolId,
}

impl Matcher<'_> {
    fn is(&self, expr: &Expression<'_>, symbol: SymbolId) -> bool {
        is_identifier_of(expr.without_parentheses(), self.scoping, symbol)
    }

    fn assigned(&self, target: &AssignmentTarget<'_>) -> Option<SymbolId> {
        match target {
            AssignmentTarget::AssignmentTargetIdentifier(id) => reference_symbol(self.scoping, id),
            _ => None,
        }
    }

    /// `symbol = <rhs>`, as an expression statement.
    fn assignment<'e, 'a>(&self, stmt: &'e Statement<'a>, symbol: SymbolId) -> Option<&'e Expression<'a>> {
        let Statement::ExpressionStatement(stmt) = stmt else {
            return None;
        };
        let Expression::AssignmentExpression(assign) = &stmt.expression else {
            return None;
        };
        (assign.operator == AssignmentOperator::Assign && self.assigned(&assign.left) == Some(symbol))
            .then_some(&assign.right)
    }

    /// `table[index] = <rhs>`, as an expression statement.
    fn element_assignment<'e, 'a>(
        &self,
        stmt: &'e Statement<'a>,
        table: SymbolId,
        index: SymbolId,
    ) -> Option<&'e Expression<'a>> {
        let Statement::ExpressionStatement(stmt) = stmt else {
            return None;
        };
        let Expression::AssignmentExpression(assign) = &stmt.expression else {
            return None;
        };
        let AssignmentTarget::ComputedMemberExpression(member) = &assign.left else {
            return None;
        };
        (assign.operator == AssignmentOperator::Assign
            && !member.optional
            && self.is(&member.object, table)
            && self.is(&member.expression, index))
        .then_some(&assign.right)
    }

    /// `result += <part>` or `result = result + <part>`.
    fn append<'e, 'a>(&self, stmt: &'e Statement<'a>, result: SymbolId) -> Option<&'e Expression<'a>> {
        let Statement::ExpressionStatement(stmt) = stmt else {
            return None;
        };
        let Expression::AssignmentExpression(assign) = &stmt.expression else {
            return None;
        };
        if self.assigned(&assign.left) != Some(result) {
            return None;
        }
        match assign.operator {
            AssignmentOperator::Addition => Some(&assign.right),
            AssignmentOperator::Assign => {
                let Expression::BinaryExpression(sum) = &assign.right else {
                    return None;
                };
                (sum.operator == BinaryOperator::Addition && self.is(&sum.left, result)).then_some(&sum.right)
            }
            _ => None,
        }
    }

    fn binary<'e, 'a>(&self, expr: &'e Expression<'a>, operator: BinaryOperator) -> Option<&'e BinaryExpression<'a>> {
        match expr.without_parentheses() {
            Expression::BinaryExpression(binary) if binary.operator == operator => Some(binary),
            _ => None,
        }
    }

    fn is_number(expr: &Expression<'_>, value: f64) -> bool {
        matches!(expr.without_parentheses(), Expression::NumericLiteral(lit) if lit.value == value)
    }

    /// `<inner> % 256`
    fn mod_256<'e, 'a>(&self, expr: &'e Expression<'a>) -> Option<&'e Expression<'a>> {
        let binary = self.binary(expr, BinaryOperator::Remainder)?;
        Self::is_number(&binary.right, 256.0).then_some(&binary.left)
    }

    /// `table[index]`
    fn element(&self, expr: &Expression<'_>, table: SymbolId, index: SymbolId) -> bool {
        matches!(expr.without_parentheses(), Expression::ComputedMemberExpression(member)
            if !member.optional && self.is(&member.object, table) && self.is(&member.expression, index))
    }

    /// `string.length`
    fn length_of(&self, expr: &Expression<'_>, string: SymbolId) -> bool {
        matches!(expr.without_parentheses(), Expression::StaticMemberExpression(member)
            if !member.optional && member.property.name == "length" && self.is(&member.object, string))
    }

    /// `string.charCodeAt(<index>)`, returning the index argument.
    fn char_code_at<'e, 'a>(&self, expr: &'e Expression<'a>, string: SymbolId) -> Option<&'e Expression<'a>> {
        let Expression::CallExpression(call) = expr.without_parentheses() else {
            return None;
        };
        let [index] = call.arguments.as_slice() else {
            return None;
        };
        is_method_call(call, self.scoping, string, "charCodeAt")
            .then(|| index.as_expression())
            .flatten()
    }

    /// `key.charCodeAt(index % key.length)`
    fn cycled_key_code(&self, expr: &Expression<'_>, key: SymbolId, index: SymbolId) -> bool {
        self.char_code_at(expr, key)
            .and_then(|arg| self.binary(arg, BinaryOperator::Remainder))
            .is_some_and(|rem| self.is(&rem.left, index) && self.length_of(&rem.right, key))
    }

    /// `String.fromCharCode(<code>)` with the global `String`.
    fn string_from_char_code<'e, 'a>(&self, expr: &'e Expression<'a>) -> Option<&'e Expression<'a>> {
        let Expression::CallExpression(call) = expr.without_parentheses() else {
            return None;
        };
        let Expression::StaticMemberExpression(member) = &call.callee else {
            return None;
        };
        let Expression::Identifier(string) = &member.object else {
            return None;
        };
        let [code] = call.arguments.as_slice() else {
            return None;
        };
        (!call.optional
            && !member.optional
            && member.property.name == "fromCharCode"
            && is_global(self.scoping, string, "String"))
        .then(|| code.as_expression())
        .flatten()
    }

    /// `<left> ^ <right>` in either order, where `is_left` picks the side it
    /// recognises; returns the other side.
    fn xor_with<'e, 'a>(
        &self,
        expr: &'e Expression<'a>,
        is_left: impl Fn(&Expression<'a>) -> bool,
    ) -> Option<&'e Expression<'a>> {
        let xor = self.binary(expr, BinaryOperator::BitwiseXOR)?;
        if is_left(&xor.left) {
            Some(&xor.right)
        } else if is_left(&xor.right) {
            Some(&xor.left)
        } else {
            None
        }
    }

    /// The leading `var`/`let` declarations of a helper body, sorted by role.
    /// Returns the statements after them.
    fn locals<'b, 'a>(&self, body: &'b [Statement<'a>], locals: &mut Locals) -> Option<&'b [Statement<'a>]> {
        let mut rest = body;
        while let [Statement::VariableDeclaration(decl), tail @ ..] = rest {
            self.declare(decl, locals)?;
            rest = tail;
        }
        Some(rest)
    }

    fn declare(&self, decl: &VariableDeclaration<'_>, locals: &mut Locals) -> Option<()> {
        if !matches!(decl.kind, VariableDeclarationKind::Var | VariableDeclarationKind::Let) {
            return None;
        }
        for declarator in &decl.declarations {
            let symbol = binding_symbol(&declarator.id)?;
            let slot = match declarator.init.as_ref().map(Expression::without_parentheses) {
                None => {
                    locals.uninit.push(symbol);
                    continue;
                }
                Some(Expression::StringLiteral(lit)) if lit.value.is_empty() => &mut locals.result,
                Some(Expression::ArrayExpression(array)) if array.elements.is_empty() => &mut locals.table,
                Some(init) if Self::is_number(init, 0.0) => &mut locals.zero,
                Some(_) => return None,
            };
            if slot.replace(symbol).is_some() {
                return None;
            }
        }
        Some(())
    }

    /// `for (<counter> = 0; <counter> < <bound>; <counter>++) <body>`, where
    /// the counter is declared by the init (`var`/`let`) or is one of
    /// `declared`. Returns the counter and the body's statements.
    fn counting_loop<'b, 'a>(
        &self,
        stmt: &'b Statement<'a>,
        declared: &[SymbolId],
        is_bound: impl Fn(&Expression<'a>) -> bool,
    ) -> Option<(SymbolId, &'b [Statement<'a>])> {
        let Statement::ForStatement(for_stmt) = stmt else {
            return None;
        };
        let counter = match for_stmt.init.as_ref()? {
            ForStatementInit::VariableDeclaration(decl)
                if matches!(decl.kind, VariableDeclarationKind::Var | VariableDeclarationKind::Let) =>
            {
                let [declarator] = decl.declarations.as_slice() else {
                    return None;
                };
                let init = declarator.init.as_ref()?;
                if !Self::is_number(init, 0.0) {
                    return None;
                }
                binding_symbol(&declarator.id)?
            }
            ForStatementInit::AssignmentExpression(assign) => {
                let counter = self.assigned(&assign.left)?;
                if assign.operator != AssignmentOperator::Assign
                    || !declared.contains(&counter)
                    || !Self::is_number(&assign.right, 0.0)
                {
                    return None;
                }
                counter
            }
            _ => return None,
        };
        let test = self.binary(for_stmt.test.as_ref()?, BinaryOperator::LessThan)?;
        if !self.is(&test.left, counter)
            || !is_bound(&test.right)
            || !self.increments(for_stmt.update.as_ref()?, counter)
        {
            return None;
        }
        let body = match &for_stmt.body {
            Statement::BlockStatement(block) => block.body.as_slice(),
            body => std::slice::from_ref(body),
        };
        Some((counter, body))
    }

    /// `counter++`, `++counter`, `counter += 1` or `counter = counter + 1`.
    fn increments(&self, expr: &Expression<'_>, counter: SymbolId) -> bool {
        match expr {
            Expression::UpdateExpression(update) => {
                update.operator == UpdateOperator::Increment
                    && matches!(&update.argument, SimpleAssignmentTarget::AssignmentTargetIdentifier(id)
                        if reference_symbol(self.scoping, id) == Some(counter))
            }
            Expression::AssignmentExpression(assign) if self.assigned(&assign.left) == Some(counter) => {
                match assign.operator {
                    AssignmentOperator::Addition => Self::is_number(&assign.right, 1.0),
                    AssignmentOperator::Assign => self
                        .binary(&assign.right, BinaryOperator::Addition)
                        .is_some_and(|sum| self.is(&sum.left, counter) && Self::is_number(&sum.right, 1.0)),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn returns(&self, stmt: &Statement<'_>, symbol: SymbolId) -> bool {
        matches!(stmt, Statement::ReturnStatement(ret)
            if ret.argument.as_ref().is_some_and(|arg| self.is(arg, symbol)))
    }

    /// ```js
    /// var r = '';
    /// for (var i = 0; i < s.length; i++) r += String.fromCharCode(s.charCodeAt(i) ^ KEY);
    /// return r;
    /// ```
    /// with `KEY` either `k.charCodeAt(i % k.length)` or `k`.
    fn match_xor(&self, body: &[Statement<'_>], value: SymbolId, key: SymbolId) -> Option<KeyForm> {
        let mut locals = Locals::default();
        let [for_stmt, ret] = self.locals(body, &mut locals)? else {
            return None;
        };
        let result = locals.result?;
        if locals.table.is_some() || locals.zero.is_some() || locals.uninit.len() > 1 {
            return None;
        }
        let (counter, [step]) = self.counting_loop(for_stmt, &locals.uninit, |bound| self.length_of(bound, value))?
        else {
            return None;
        };
        let roles = [value, key, result, counter];
        if !distinct(&roles) || !self.returns(ret, result) {
            return None;
        }
        let code = self.string_from_char_code(self.append(step, result)?)?;
        let key_code = self.xor_with(code, |side| {
            self.char_code_at(side, value)
                .is_some_and(|index| self.is(index, counter))
        })?;
        if self.cycled_key_code(key_code, key, counter) {
            Some(KeyForm::String)
        } else if self.is(key_code, key) {
            Some(KeyForm::Number)
        } else {
            None
        }
    }

    /// ```js
    /// var s = [], j = 0, x, r = '';
    /// d = atob(d);                                  // optional
    /// for (var i = 0; i < 256; i++) s[i] = i;
    /// for (i = 0; i < 256; i++) { j = (j + s[i] + k.charCodeAt(i % k.length)) % 256; SWAP }
    /// i = 0; j = 0;
    /// for (var y = 0; y < d.length; y++) {
    ///     i = (i + 1) % 256; j = (j + s[i]) % 256; SWAP
    ///     r += String.fromCharCode(d.charCodeAt(y) ^ s[(s[i] + s[j]) % 256]);
    /// }
    /// return r;
    /// ```
    /// where `SWAP` is `x = s[i]; s[i] = s[j]; s[j] = x;`. Returns whether the
    /// helper decodes base64 first.
    fn match_rc4(&self, body: &[Statement<'_>], value: SymbolId, key: SymbolId) -> Option<bool> {
        let mut locals = Locals::default();
        let mut rest = self.locals(body, &mut locals)?;
        let (table, j, result) = (locals.table?, locals.zero?, locals.result?);
        let base64 = if let [first, tail @ ..] = rest
            && let Some(decoded) = self.assignment(first, value)
        {
            let Expression::CallExpression(call) = decoded.without_parentheses() else {
                return None;
            };
            let Expression::Identifier(callee) = &call.callee else {
                return None;
            };
            let [Argument::Identifier(arg)] = call.arguments.as_slice() else {
                return None;
            };
            if call.optional
                || !is_global(self.scoping, callee, "atob")
                || reference_symbol(self.scoping, arg) != Some(value)
            {
                return None;
            }
            rest = tail;
            true
        } else {
            false
        };
        let [init_loop, schedule_loop, reset_i, reset_j, stream_loop, returned] = rest else {
            return None;
        };
        let vars = Rc4Vars {
            value,
            key,
            table,
            j,
            result,
        };
        let (i, x) = self.rc4_key_schedule(init_loop, schedule_loop, &locals.uninit, &vars)?;
        if !self.assignment(reset_i, i).is_some_and(|rhs| Self::is_number(rhs, 0.0))
            || !self.assignment(reset_j, j).is_some_and(|rhs| Self::is_number(rhs, 0.0))
        {
            return None;
        }
        let mut declared = locals.uninit;
        declared.push(i);
        let y = self.rc4_stream(stream_loop, &declared, &vars, i, x)?;
        let roles = [value, key, table, j, result, x, i, y];
        (distinct(&roles) && self.returns(returned, result)).then_some(base64)
    }

    /// The fill loop `for (var i = 0; i < 256; i++) s[i] = i;` and the key
    /// schedule `for (i = 0; i < 256; i++) { j = (j + s[i] + k.charCodeAt(i %
    /// k.length)) % 256; SWAP }`. Returns `i` and the swap's temporary.
    fn rc4_key_schedule(
        &self,
        init_loop: &Statement<'_>,
        schedule_loop: &Statement<'_>,
        uninit: &[SymbolId],
        vars: &Rc4Vars,
    ) -> Option<(SymbolId, SymbolId)> {
        let Rc4Vars { key, table, j, .. } = *vars;
        let is_256 = |bound: &Expression<'_>| Self::is_number(bound, 256.0);
        let (i, [fill]) = self.counting_loop(init_loop, uninit, is_256)? else {
            return None;
        };
        if !self
            .element_assignment(fill, table, i)
            .is_some_and(|rhs| self.is(rhs, i))
        {
            return None;
        }
        let declared = [uninit, &[i]].concat();
        let (schedule_i, [schedule, swap @ ..]) = self.counting_loop(schedule_loop, &declared, is_256)? else {
            return None;
        };
        let x = self.swap(swap, table, i, j).filter(|x| uninit.contains(x))?;
        let schedule_ok = self
            .assignment(schedule, j)
            .and_then(|rhs| self.mod_256(rhs))
            .and_then(|sum| self.binary(sum, BinaryOperator::Addition))
            .is_some_and(|sum| {
                self.cycled_key_code(&sum.right, key, i)
                    && self
                        .binary(&sum.left, BinaryOperator::Addition)
                        .is_some_and(|inner| self.is(&inner.left, j) && self.element(&inner.right, table, i))
            });
        (schedule_i == i && schedule_ok).then_some((i, x))
    }

    /// `for (var y = 0; y < d.length; y++) { i = (i + 1) % 256; j = (j +
    /// s[i]) % 256; SWAP r += String.fromCharCode(d.charCodeAt(y) ^ s[(s[i] +
    /// s[j]) % 256]); }`. Returns `y`.
    fn rc4_stream(
        &self,
        stream_loop: &Statement<'_>,
        declared: &[SymbolId],
        vars: &Rc4Vars,
        i: SymbolId,
        x: SymbolId,
    ) -> Option<SymbolId> {
        let Rc4Vars {
            value,
            table,
            j,
            result,
            ..
        } = *vars;
        let (y, [step_i, step_j, stream @ ..]) =
            self.counting_loop(stream_loop, declared, |bound| self.length_of(bound, value))?
        else {
            return None;
        };
        let [swap @ .., emit] = stream else {
            return None;
        };
        let advances_i = self
            .assignment(step_i, i)
            .and_then(|rhs| self.mod_256(rhs))
            .and_then(|sum| self.binary(sum, BinaryOperator::Addition))
            .is_some_and(|sum| self.is(&sum.left, i) && Self::is_number(&sum.right, 1.0));
        let mixes_j = self
            .assignment(step_j, j)
            .and_then(|rhs| self.mod_256(rhs))
            .and_then(|sum| self.binary(sum, BinaryOperator::Addition))
            .is_some_and(|sum| self.is(&sum.left, j) && self.element(&sum.right, table, i));
        let emit_ok = self
            .append(emit, result)
            .and_then(|part| self.string_from_char_code(part))
            .and_then(|code| {
                self.xor_with(code, |side| {
                    self.char_code_at(side, value).is_some_and(|index| self.is(index, y))
                })
            })
            .is_some_and(|keystream| self.keystream(keystream, table, i, j));
        (advances_i && mixes_j && emit_ok && self.swap(swap, table, i, j) == Some(x)).then_some(y)
    }

    /// `x = s[i]; s[i] = s[j]; s[j] = x;`, returning `x`.
    fn swap(&self, stmts: &[Statement<'_>], table: SymbolId, i: SymbolId, j: SymbolId) -> Option<SymbolId> {
        let [save, first, second] = stmts else {
            return None;
        };
        let Statement::ExpressionStatement(save) = save else {
            return None;
        };
        let Expression::AssignmentExpression(assign) = &save.expression else {
            return None;
        };
        let x = self.assigned(&assign.left)?;
        let ok = assign.operator == AssignmentOperator::Assign
            && self.element(&assign.right, table, i)
            && self
                .element_assignment(first, table, i)
                .is_some_and(|rhs| self.element(rhs, table, j))
            && self
                .element_assignment(second, table, j)
                .is_some_and(|rhs| self.is(rhs, x));
        ok.then_some(x)
    }

    /// `s[(s[i] + s[j]) % 256]`
    fn keystream(&self, expr: &Expression<'_>, table: SymbolId, i: SymbolId, j: SymbolId) -> bool {
        let Expression::ComputedMemberExpression(member) = expr.without_parentheses() else {
            return false;
        };
        !member.optional
            && self.is(&member.object, table)
            && self
                .mod_256(&member.expression)
                .and_then(|sum| self.binary(sum, BinaryOperator::Addition))
                .is_some_and(|sum| self.element(&sum.left, table, i) && self.element(&sum.right, table, j))
    }
}

pub(super) fn distinct(symbols: &[SymbolId]) -> bool {
    symbols
        .iter()
        .enumerate()
        .all(|(n, symbol)| !symbols[..n].contains(symbol))
}
