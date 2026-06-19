use oxc_ast::ast::{Expression, Statement};
use oxc_ast_visit::{Visit, walk};

pub(super) struct AstNormalizer {
    pub(super) output: String,
    depth: usize,
    max_depth: usize,
}

impl AstNormalizer {
    pub(super) const fn new(max_depth: usize) -> Self {
        Self {
            output: String::new(),
            depth: 0,
            max_depth,
        }
    }

    fn push(&mut self, s: &str) {
        self.output.push_str(s);
    }
}

impl<'a> Visit<'a> for AstNormalizer {
    fn visit_statement(&mut self, stmt: &Statement<'a>) {
        if self.depth >= self.max_depth {
            return;
        }
        self.depth += 1;

        match stmt {
            Statement::BlockStatement(_) => self.push("Block"),
            Statement::BreakStatement(_) => self.push("Break"),
            Statement::ContinueStatement(_) => self.push("Continue"),
            Statement::DebuggerStatement(_) => self.push("Debugger"),
            Statement::DoWhileStatement(_) => self.push("DoWhile"),
            Statement::EmptyStatement(_) => self.push("Empty"),
            Statement::ExpressionStatement(_) => self.push("Expr"),
            Statement::ForInStatement(_) => self.push("ForIn"),
            Statement::ForOfStatement(_) => self.push("ForOf"),
            Statement::ForStatement(_) => self.push("For"),
            Statement::IfStatement(_) => self.push("If"),
            Statement::LabeledStatement(_) => self.push("Labeled"),
            Statement::ReturnStatement(_) => self.push("Return"),
            Statement::SwitchStatement(_) => self.push("Switch"),
            Statement::ThrowStatement(_) => self.push("Throw"),
            Statement::TryStatement(_) => self.push("Try"),
            Statement::WhileStatement(_) => self.push("While"),
            Statement::WithStatement(_) => self.push("With"),
            Statement::VariableDeclaration(_) => self.push("VarDecl"),
            Statement::FunctionDeclaration(_) => self.push("FuncDecl"),
            Statement::ClassDeclaration(_) => self.push("ClassDecl"),
            Statement::ImportDeclaration(_) => self.push("Import"),
            Statement::ExportAllDeclaration(_) => self.push("ExportAll"),
            Statement::ExportDefaultDeclaration(_) => self.push("ExportDefault"),
            Statement::ExportNamedDeclaration(_) => self.push("ExportNamed"),
            Statement::TSTypeAliasDeclaration(_) => self.push("TSTypeAlias"),
            Statement::TSInterfaceDeclaration(_) => self.push("TSInterface"),
            Statement::TSEnumDeclaration(_) => self.push("TSEnum"),
            Statement::TSModuleDeclaration(_) => self.push("TSModule"),
            Statement::TSImportEqualsDeclaration(_) => self.push("TSImportEquals"),
            Statement::TSExportAssignment(_) => self.push("TSExportAssign"),
            Statement::TSNamespaceExportDeclaration(_) => self.push("TSNsExport"),
            Statement::TSGlobalDeclaration(_) => self.push("TSGlobal"),
        }

        walk::walk_statement(self, stmt);
        self.depth -= 1;
    }

    fn visit_expression(&mut self, expr: &Expression<'a>) {
        if self.depth >= self.max_depth {
            return;
        }
        self.depth += 1;

        match expr {
            Expression::BooleanLiteral(_) => self.push("Bool"),
            Expression::NullLiteral(_) => self.push("Null"),
            Expression::NumericLiteral(_) => self.push("Num"),
            Expression::BigIntLiteral(_) => self.push("BigInt"),
            Expression::RegExpLiteral(_) => self.push("RegExp"),
            Expression::StringLiteral(_) => self.push("Str"),
            Expression::TemplateLiteral(_) => self.push("Template"),
            Expression::Identifier(_) => self.push("Id"),
            Expression::MetaProperty(_) => self.push("Meta"),
            Expression::Super(_) => self.push("Super"),
            Expression::ArrayExpression(_) => self.push("Array"),
            Expression::ArrowFunctionExpression(_) => self.push("Arrow"),
            Expression::AssignmentExpression(_) => self.push("Assign"),
            Expression::AwaitExpression(_) => self.push("Await"),
            Expression::BinaryExpression(e) => {
                self.push("Bin");
                self.push(e.operator.as_str());
            }
            Expression::CallExpression(_) => self.push("Call"),
            Expression::ChainExpression(_) => self.push("Chain"),
            Expression::ClassExpression(_) => self.push("Class"),
            Expression::ConditionalExpression(_) => self.push("Cond"),
            Expression::FunctionExpression(_) => self.push("Func"),
            Expression::ImportExpression(_) => self.push("DynImport"),
            Expression::LogicalExpression(e) => {
                self.push("Logic");
                self.push(e.operator.as_str());
            }
            Expression::NewExpression(_) => self.push("New"),
            Expression::ObjectExpression(_) => self.push("Obj"),
            Expression::ParenthesizedExpression(_) => self.push("Paren"),
            Expression::SequenceExpression(_) => self.push("Seq"),
            Expression::TaggedTemplateExpression(_) => self.push("TaggedTemplate"),
            Expression::ThisExpression(_) => self.push("This"),
            Expression::UnaryExpression(e) => {
                self.push("Unary");
                self.push(e.operator.as_str());
            }
            Expression::UpdateExpression(e) => {
                self.push("Update");
                self.push(e.operator.as_str());
            }
            Expression::YieldExpression(_) => self.push("Yield"),
            Expression::PrivateInExpression(_) => self.push("PrivateIn"),
            Expression::StaticMemberExpression(_) => self.push("StaticMember"),
            Expression::ComputedMemberExpression(_) => self.push("ComputedMember"),
            Expression::PrivateFieldExpression(_) => self.push("PrivateField"),
            Expression::TSAsExpression(_) => self.push("TSAs"),
            Expression::TSSatisfiesExpression(_) => self.push("TSSatisfies"),
            Expression::TSTypeAssertion(_) => self.push("TSTypeAssertion"),
            Expression::TSNonNullExpression(_) => self.push("TSNonNull"),
            Expression::TSInstantiationExpression(_) => self.push("TSInstantiation"),
            Expression::JSXElement(_) => self.push("JSXElement"),
            Expression::JSXFragment(_) => self.push("JSXFragment"),
            Expression::V8IntrinsicExpression(_) => self.push("V8Intrinsic"),
        }

        walk::walk_expression(self, expr);
        self.depth -= 1;
    }
}
