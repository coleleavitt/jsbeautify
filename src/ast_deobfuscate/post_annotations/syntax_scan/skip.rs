pub(in crate::ast_deobfuscate::post_annotations) fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while matches!(bytes.get(i), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        i += 1;
    }
    i
}

pub(super) fn skip_quoted(bytes: &[u8], mut i: usize, quote: u8) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

pub(super) fn skip_template(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'$' if bytes.get(i + 1) == Some(&b'{') => i = skip_template_expr(bytes, i + 2),
            b'`' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

pub(super) fn skip_template_expr(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => i = skip_quoted(bytes, i, bytes[i]),
            b'`' => i = skip_template(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = skip_block_comment(bytes, i),
            b'/' if is_regex_literal_start(bytes, i) => i = skip_regex_literal(bytes, i),
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

pub(super) const fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
    i += 2;
    while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
        i += 1;
    }
    i
}

pub(super) const fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
    i += 2;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

const fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn regex_prefix_keyword_before(bytes: &[u8], mut i: usize) -> bool {
    while i > 0 && matches!(bytes.get(i - 1), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        i -= 1;
    }
    if i == 0 || !is_ident_continue(bytes[i - 1]) {
        return false;
    }

    let end = i;
    while i > 0 && is_ident_continue(bytes[i - 1]) {
        i -= 1;
    }
    matches!(
        std::str::from_utf8(&bytes[i..end]).ok(),
        Some("return" | "throw" | "case" | "delete" | "void" | "typeof" | "yield" | "await" | "in" | "instanceof")
    )
}

pub(super) fn is_regex_literal_start(bytes: &[u8], i: usize) -> bool {
    if bytes.get(i) != Some(&b'/') || matches!(bytes.get(i + 1), Some(b'/' | b'*')) {
        return false;
    }
    if regex_prefix_keyword_before(bytes, i) {
        return true;
    }
    let mut prev = i;
    while prev > 0 && matches!(bytes.get(prev - 1), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        prev -= 1;
    }
    prev == 0
        || matches!(
            bytes[prev - 1],
            b'(' | b'['
                | b'{'
                | b'='
                | b':'
                | b','
                | b';'
                | b'!'
                | b'?'
                | b'&'
                | b'|'
                | b'+'
                | b'-'
                | b'*'
                | b'%'
                | b'~'
                | b'^'
                | b'<'
                | b'>'
        )
}

pub(super) fn skip_regex_literal(bytes: &[u8], mut i: usize) -> usize {
    let start = i;
    let mut in_class = false;
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i = (i + 2).min(bytes.len()),
            b'[' if !in_class => {
                in_class = true;
                i += 1;
            }
            b']' if in_class => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return i;
            }
            b'\n' | b'\r' => return start + 1,
            _ => i += 1,
        }
    }
    start + 1
}
