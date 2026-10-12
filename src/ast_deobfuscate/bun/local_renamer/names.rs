use oxc_syntax::scope::ScopeId;
use rustc_hash::{FxHashMap, FxHashSet};

/// A name is a single-character Bun-mangled local: one ASCII letter, `$`, or `_`.
pub(super) const fn is_single_char_mangled(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 1 && (b[0].is_ascii_alphabetic() || b[0] == b'$' || b[0] == b'_')
}

/// Idiomatic short names worth keeping even in readable mode — math/graphics/
/// loop identifiers a positional/usage rename would only make worse. Includes
/// `i`/`j`/`k` so loop-counter proposals are idempotent across re-runs.
const MEANINGFUL_SHORT: &[&str] = &[
    "i", "j", "k", "x", "y", "z", "w", "h", "id", "el", "db", "fs", "os", "fn", "cb", "ok", "ip", "ua", "px", "dx",
    "dy", "rgb", "rgba",
];

#[must_use]
pub fn is_meaningful_short(name: &str) -> bool {
    MEANINGFUL_SHORT.contains(&name)
}

/// Looks compiler/minifier-generated (esbuild/webpack/Bun aliases).
///
/// Matches one or two bare letters, OR ≤3 letters with trailing digits, OR a
/// `_`/`$` prefix. Deliberately broad; [`is_meaningful_short`] is applied
/// separately so idiomatic short names survive.
#[must_use]
pub fn is_likely_generated_alias(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 5 {
        return false;
    }
    let stripped = name.trim_start_matches(['_', '$']);
    let had_prefix = stripped.len() != name.len();
    if stripped.is_empty() {
        return name.len() <= 2; // "_", "$", "__"
    }
    let letters = stripped.chars().take_while(char::is_ascii_alphabetic).count();
    if letters == 0 {
        return false;
    }
    let rest = &stripped[letters..];
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    // 1-2 bare letters (`a`, `ab`).
    if letters <= 2 && rest.is_empty() && !had_prefix {
        return true;
    }
    // ≤3 letters WITH trailing digits or a `_`/`$` prefix (`ab1`, `_a2`, `$ref`).
    letters <= 3 && (!rest.is_empty() || had_prefix)
}

/// A base name usable as a readable identifier: valid JS identifier, not a
/// reserved word.
#[must_use]
pub fn is_usable_base(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') && !is_reserved_ident(name)
}

fn is_reserved_ident(name: &str) -> bool {
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
            | "enum"
            | "await"
            | "arguments"
            | "eval"
    )
}

/// First non-blocked variant of `base`: `base`, `base2`, … `base9`, else `None`.
pub(super) fn pick_readable<F>(base: &str, mut is_blocked: F) -> Option<String>
where
    F: FnMut(&str) -> bool,
{
    for n in 0..=9 {
        let cand = if n == 0 { base.to_string() } else { format!("{base}{n}") };
        if !is_blocked(&cand) {
            return Some(cand);
        }
    }
    None
}

pub(super) fn is_blocked_name(
    name: &str,
    current_name: &str,
    local: &FxHashSet<String>,
    subtree_names: Option<&FxHashSet<&str>>,
    free_names: Option<&FxHashSet<&str>>,
    ancestor_scopes: &[ScopeId],
    scope_name_sets: &FxHashMap<ScopeId, FxHashSet<&str>>,
    assigned: &FxHashMap<ScopeId, FxHashSet<String>>,
) -> bool {
    if name == current_name {
        return local.contains(name);
    }
    if local.contains(name)
        || subtree_names.is_some_and(|names| names.contains(name))
        || free_names.is_some_and(|names| names.contains(name))
    {
        return true;
    }
    ancestor_scopes.iter().any(|anc| {
        scope_name_sets.get(anc).is_some_and(|names| names.contains(name))
            || assigned.get(anc).is_some_and(|names| names.contains(name))
    })
}
