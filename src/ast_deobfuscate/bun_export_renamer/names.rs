/// JS reserved words + dangerous globals that must never become a binding name
/// (renaming a symbol to one of these produces invalid JavaScript, e.g.
/// `var instanceof = ...` from a Zod `z.instanceof` export key).
pub(super) fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "let"
            | "static"
            | "implements"
            | "interface"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "await"
            | "async"
            | "undefined"
            | "arguments"
            | "eval"
            | "NaN"
            | "Infinity"
            | "globalThis"
    )
}

pub(super) const fn is_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == '$'
}

pub(super) const fn is_identifier_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

pub fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    is_identifier_start(first) && chars.all(is_identifier_continue)
}

pub fn sanitize_identifier(name: &str) -> String {
    let mut out = String::with_capacity(name.len().max("exported".len()));
    for ch in name.chars() {
        if out.is_empty() {
            if is_identifier_start(ch) {
                out.push(ch);
            } else if is_identifier_continue(ch) {
                out.push('_');
                out.push(ch);
            } else {
                out.push('_');
            }
        } else if is_identifier_continue(ch) {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        out.push_str("exported");
    }
    out
}

/// Names that are too generic to be useful when applied as renames.
pub(super) fn is_generic_name(name: &str) -> bool {
    matches!(
        name,
        "default"
            | "init"
            | "get"
            | "set"
            | "call"
            | "apply"
            | "bind"
            | "then"
            | "catch"
            | "finally"
            | "next"
            | "done"
            | "value"
            | "run"
            | "exec"
            | "start"
            | "stop"
            | "end"
            | "new"
            | "create"
    )
}
