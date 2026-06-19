pub(super) fn apply_replacements(code: &str, replacements: &mut [(u32, u32, String)]) -> String {
    if replacements.is_empty() {
        return code.to_string();
    }

    // Sort by start position ascending for single-pass construction
    replacements.sort_by_key(|(start, _, _)| *start);

    // Estimate capacity: original size + some slack for longer replacements
    let mut result = String::with_capacity(code.len() + code.len() / 10);
    let code_bytes = code.as_bytes();
    let mut cursor = 0usize;

    for (start, end, replacement) in replacements.iter() {
        let start = *start as usize;
        let end = *end as usize;

        // Skip invalid or overlapping replacements
        if start < cursor || end > code_bytes.len() || start >= end {
            continue;
        }

        // Copy bytes from cursor to replacement start
        if start > cursor {
            // SAFETY: we're iterating through valid UTF-8 boundaries from the AST
            result.push_str(unsafe { std::str::from_utf8_unchecked(&code_bytes[cursor..start]) });
        }

        // Add replacement
        result.push_str(replacement);
        cursor = end;
    }

    // Copy remaining bytes after last replacement
    if cursor < code_bytes.len() {
        result.push_str(unsafe { std::str::from_utf8_unchecked(&code_bytes[cursor..]) });
    }

    result
}

#[derive(Debug, Clone)]
pub struct AlignmentStats {
    pub source_statements: usize,
    pub target_statements: usize,
    pub matched_statements: usize,
    pub source_replacements: usize,
    pub target_replacements: usize,
    pub canonical_names_generated: usize,
    pub stable_names_used: usize,
}

impl AlignmentStats {
    #[must_use]
    pub fn match_rate(&self) -> f64 {
        if self.source_statements == 0 {
            return 0.0;
        }
        (self.matched_statements as f64 / self.source_statements as f64) * 100.0
    }
}

pub(super) fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        "await"
            | "break"
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
            | "let"
            | "new"
            | "null"
            | "return"
            | "static"
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
            | "implements"
            | "interface"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "arguments"
            | "eval"
            | "undefined"
            | "NaN"
            | "Infinity"
    )
}
