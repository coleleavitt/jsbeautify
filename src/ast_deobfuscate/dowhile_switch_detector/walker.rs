use oxc_ast::ast::{BindingPattern, BlockStatement, Expression, FunctionBody, Program, Statement};
use oxc_ast_visit::{Visit, walk};
use oxc_syntax::symbol::SymbolId;

use super::bindings::{Bindings, Constants, Position};
use super::extraction::extract;
use super::types::DoWhileDispatcher;
use super::value::evaluate;

/// A dispatcher found in a statement list, before its call sites are known.
pub(super) struct Candidate {
    /// The binding the function expression is assigned to.
    pub symbol: SymbolId,
    /// The function expression's own name, which its body can call it by.
    pub name: Option<SymbolId>,
    pub declared: Position,
    pub dispatcher: DoWhileDispatcher,
}

/// Records, per statement list, the constants its declarations define and
/// the dispatchers they assign.
pub(super) struct Walker<'b, 's> {
    bindings: &'b Bindings<'s>,
    lists: usize,
    pub constants: Constants,
    pub candidates: Vec<Candidate>,
}

impl<'b, 's> Walker<'b, 's> {
    pub fn new(bindings: &'b Bindings<'s>) -> Self {
        Self {
            bindings,
            lists: 0,
            constants: Constants::default(),
            candidates: Vec::new(),
        }
    }

    fn scan(&mut self, statements: &[Statement<'_>]) {
        let list = self.lists;
        self.lists += 1;
        for (statement, s) in statements.iter().enumerate() {
            let Statement::VariableDeclaration(declaration) = s else {
                continue;
            };
            for (declarator, d) in declaration.declarations.iter().enumerate() {
                let (BindingPattern::BindingIdentifier(id), Some(init)) = (&d.id, &d.init) else {
                    continue;
                };
                let Some(symbol) = id.symbol_id.get() else {
                    continue;
                };
                let declared = Position {
                    list,
                    statement,
                    declarator,
                };
                if let Expression::FunctionExpression(func) = init {
                    let bindings = self.bindings;
                    if bindings.is_tracked(symbol)
                        && !bindings.is_written(symbol)
                        && !bindings.is_shared(symbol)
                        && let Some(dispatcher) = extract(func, bindings, &self.constants, declared)
                    {
                        self.candidates.push(Candidate {
                            symbol,
                            name: func.id.as_ref().and_then(|id| id.symbol_id.get()),
                            declared,
                            dispatcher,
                        });
                    }
                } else if self.bindings.is_constant(symbol)
                    && let Some(value) = evaluate(init, &|id| self.constants.lookup(self.bindings, id, declared))
                {
                    self.constants.insert(symbol, declared, value);
                }
            }
        }
    }
}

impl<'a> Visit<'a> for Walker<'_, '_> {
    fn visit_program(&mut self, it: &Program<'a>) {
        self.scan(&it.body);
        walk::walk_program(self, it);
    }

    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        self.scan(&it.statements);
        walk::walk_function_body(self, it);
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.scan(&it.body);
        walk::walk_block_statement(self, it);
    }
}
