use oxc_ast::ast::{Expression, Statement};

/// Find the decoder function name referenced inside the IIFE body.
/// Does NOT require knowing which names are decoders — just finds `const d = SOME_NAME;`
/// or the function name used in `parseInt(NAME(...))` calls.
pub(super) fn find_decoder_ref_in_body(stmts: &[Statement<'_>]) -> Option<String> {
    // Look for `const d = SOME_NAME;` pattern
    for stmt in stmts {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init
                    && let Expression::Identifier(id) = init
                {
                    let name = id.name.as_str();
                    if name.starts_with("_0x") || name.starts_with("_0X") {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    // Scan for parseInt(NAME(...)) calls to find the decoder name
    for stmt in stmts {
        if let Some(name) = scan_stmt_for_parseint_decoder(stmt) {
            return Some(name);
        }
    }

    None
}

fn scan_stmt_for_parseint_decoder(stmt: &Statement<'_>) -> Option<String> {
    match stmt {
        Statement::WhileStatement(w) => scan_stmt_for_parseint_decoder(&w.body),
        Statement::BlockStatement(b) => {
            for s in &b.body {
                if let Some(name) = scan_stmt_for_parseint_decoder(s) {
                    return Some(name);
                }
            }
            None
        }
        Statement::TryStatement(t) => {
            for s in &t.block.body {
                if let Some(name) = scan_stmt_for_parseint_decoder(s) {
                    return Some(name);
                }
            }
            None
        }
        Statement::VariableDeclaration(v) => {
            for decl in &v.declarations {
                if let Some(init) = &decl.init
                    && let Some(name) = scan_expr_for_parseint_decoder(init)
                {
                    return Some(name);
                }
            }
            None
        }
        Statement::ExpressionStatement(e) => scan_expr_for_parseint_decoder(&e.expression),
        _ => None,
    }
}

fn scan_expr_for_parseint_decoder(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::CallExpression(call) => {
            // Check if this is parseInt(DECODER_NAME(...))
            if let Expression::Identifier(id) = &call.callee
                && id.name.as_str() == "parseInt"
                && let Some(first_arg) = call.arguments.first()
                && let Some(arg_expr) = first_arg.as_expression()
                && let Expression::CallExpression(inner) = arg_expr
                && let Expression::Identifier(inner_id) = &inner.callee
            {
                return Some(inner_id.name.as_str().to_string());
            }
            for arg in &call.arguments {
                if let Some(e) = arg.as_expression()
                    && let Some(name) = scan_expr_for_parseint_decoder(e)
                {
                    return Some(name);
                }
            }
            None
        }
        Expression::BinaryExpression(bin) => {
            scan_expr_for_parseint_decoder(&bin.left).or_else(|| scan_expr_for_parseint_decoder(&bin.right))
        }
        Expression::UnaryExpression(un) => scan_expr_for_parseint_decoder(&un.argument),
        Expression::ParenthesizedExpression(p) => scan_expr_for_parseint_decoder(&p.expression),
        _ => None,
    }
}
