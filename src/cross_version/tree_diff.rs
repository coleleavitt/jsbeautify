//! Compare two deobfuscated output trees function by function
//! (`jsbeautify diff-tree OLD NEW`).
//!
//! Two `--project` outputs of one program at two versions name most bindings
//! the same way, but code moves between chunk files from release to release,
//! and a function whose code changed gets a new shape name (`fn_…`). Pairing
//! files cannot line that up, so this compares the whole trees:
//!
//! 1. Every named function or class in either tree is indexed with its file,
//!    line, a hash of its text, and its tokens: string and template text,
//!    property names, and identifiers that are names (not minified or `vN`
//!    slots).
//! 2. Functions with the same stable name in both trees are paired; a name
//!    found in different files is a move, a different text hash a change.
//! 3. The rest are matched by content: the Jaccard similarity of their token
//!    sets, best pairs first, above a threshold. These are the functions whose
//!    code (and so whose shape name) changed, or whose minified name did.
//!
//!    What is still unmatched and changed a lot is matched weakly: a lower
//!    threshold, but only for a mutual best candidate that clearly beats the
//!    runner-up on both sides. These are reported apart from the content matches.
//!
//! The trees are only read and parsed; nothing is executed.

use std::cmp::Reverse;
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::AstKind;
use oxc_ast::ast::{
    Class, Declaration, ExportDefaultDeclarationKind, Expression, Function, Statement, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::ast_deobfuscate::default_export::is_minified;
use crate::ast_deobfuscate::project::shape::statement_shape;
use crate::project::{is_javascript, list_files, parallel_map};

/// Options of [`diff_trees`].
#[derive(Debug, Clone, Copy)]
pub struct TreeDiffOptions {
    /// Lowest token-set similarity (0–1) a content match needs.
    pub min_score: f64,
    /// Fewest tokens a function needs to be matched by content.
    pub min_tokens: usize,
    /// Lowest token-set similarity (0–1) a weak match needs.
    pub weak_min_score: f64,
    /// How much a weak match's best candidate must beat the runner-up by.
    pub weak_margin: f64,
}

impl Default for TreeDiffOptions {
    fn default() -> Self {
        Self {
            min_score: 0.5,
            min_tokens: 3,
            weak_min_score: 0.25,
            weak_margin: 0.1,
        }
    }
}

/// Where a function is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Site {
    pub name: String,
    pub file: String,
    pub line: u32,
}

/// One function of the old tree and its counterpart in the new one.
#[derive(Debug, Clone, Serialize)]
pub struct Pair {
    pub old: Site,
    pub new: Site,
    /// Token-set similarity, 1 for an unchanged function.
    pub score: f64,
    /// Its text differs between the versions.
    pub changed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TreeDiffSummary {
    pub files_old: usize,
    pub files_new: usize,
    pub functions_old: usize,
    pub functions_new: usize,
    /// Paired by name, same text.
    pub same: usize,
    /// Paired by name, text changed.
    pub changed: usize,
    /// Paired by name, out of a file the new tree still has into another
    /// (a Bun or esbuild chunk is renamed with its content hash every build,
    /// so its functions count as moved only where the old file name lives on).
    pub moved: usize,
    /// Paired by content under another name.
    pub renamed: usize,
    /// Paired weakly: a changed function whose best candidate is clearly ahead.
    pub weak_renamed: usize,
    pub added: usize,
    pub removed: usize,
}

/// What [`diff_trees`] found.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TreeDiff {
    pub summary: TreeDiffSummary,
    /// Paired by content under another name: old name -> new name.
    pub renamed: Vec<Pair>,
    /// Paired weakly (see [`TreeDiffOptions::weak_min_score`]), highest score first.
    pub weak_renamed: Vec<Pair>,
    /// Paired by name, text changed.
    pub changed: Vec<Pair>,
    /// Paired by name in another file (changed or not).
    pub moved: Vec<Pair>,
    /// Only in the new tree.
    pub added: Vec<Site>,
    /// Only in the old tree.
    pub removed: Vec<Site>,
}

/// Compare the JavaScript files under `old` and `new`.
///
/// # Errors
/// Returns an error if a directory or file cannot be read.
pub fn diff_trees(old: &Path, new: &Path, options: TreeDiffOptions) -> std::io::Result<TreeDiff> {
    let (old_files, old_functions) = index_tree(old)?;
    let (new_files, new_functions) = index_tree(new)?;
    let new_paths: FxHashSet<String> = new_functions.iter().map(|e| e.site.file.clone()).collect();
    let mut diff = match_functions(&old_functions, &new_functions, options);
    diff.moved.retain(|pair| new_paths.contains(&pair.old.file));
    diff.summary.moved = diff.moved.len();
    diff.summary.files_old = old_files;
    diff.summary.files_new = new_files;
    Ok(diff)
}

/// A named function of one tree.
#[derive(Debug, Clone)]
struct Entry {
    site: Site,
    text_hash: u64,
    tokens: Vec<String>,
}

fn index_tree(root: &Path) -> std::io::Result<(usize, Vec<Entry>)> {
    let files: Vec<_> = list_files(root)?
        .into_iter()
        .filter(|path| is_javascript(path) && !path.starts_with(".jsbeautify"))
        .collect();
    let indexed: Vec<std::io::Result<Vec<Entry>>> = parallel_map(&files, |relative| {
        let code = crate::source_text::read(&root.join(relative))?;
        let file = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        Ok(crate::stack::run_on_sized_stack(code.len(), || {
            index_source(&file, &code)
        }))
    });
    let mut entries = Vec::new();
    for file in indexed {
        entries.extend(file?);
    }
    Ok((files.len(), entries))
}

fn index_source(file: &str, code: &str) -> Vec<Entry> {
    let allocator = Allocator::default();
    let source_type = if Path::new(file)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cjs"))
    {
        SourceType::cjs()
    } else {
        SourceType::mjs()
    };
    let mut parsed = Parser::new(&allocator, code, source_type).parse();
    if parsed.fatal_error || !parsed.diagnostics.is_empty() {
        parsed = Parser::new(&allocator, code, SourceType::script()).parse();
    }
    let line_starts: Vec<u32> = std::iter::once(0)
        .chain(
            code.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .filter_map(|(i, _)| u32::try_from(i + 1).ok()),
        )
        .collect();
    let mut collector = Collector {
        file,
        source: code,
        line_starts: &line_starts,
        entries: Vec::new(),
    };
    collector.visit_program(&parsed.program);
    // Top-level statements that carry a string: a `console.log("tengu_…")`
    // call reads a flag outside any function, and its shape pairs it by name.
    for statement in &parsed.program.body {
        if !is_top_level_statement(statement) {
            continue;
        }
        let mut probe = Tokens {
            source: code,
            set: FxHashSet::default(),
        };
        probe.visit_statement(statement);
        if probe.set.iter().any(|token| token.starts_with("s:")) {
            let span = statement.span();
            collector.record(&statement_shape(code, statement), span.start, span, |tokens| {
                tokens.visit_statement(statement);
            });
        }
    }
    collector.entries
}

/// Whether a top-level statement is indexed as a `top_…` entry: not an import,
/// not an export of only specifiers, and not a function or class declaration
/// (those are indexed by name).
fn is_top_level_statement(statement: &Statement<'_>) -> bool {
    let declaration = match statement {
        Statement::ImportDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::ExportNamedDeclaration(_)
        | Statement::ExportFromDeclaration(_) => return false,
        Statement::ExportDefaultDeclaration(export) => {
            return !matches!(
                export.declaration,
                ExportDefaultDeclarationKind::FunctionDeclaration(_)
                    | ExportDefaultDeclarationKind::ClassDeclaration(_)
            );
        }
        Statement::ExportDeclaration(export) => Some(&export.declaration),
        _ => statement.as_declaration(),
    };
    !matches!(
        declaration,
        Some(Declaration::FunctionDeclaration(_) | Declaration::ClassDeclaration(_))
    )
}

/// Whether `name` carries meaning across versions: not minified, not a
/// `vN` slot.
fn is_stable_name(name: &str) -> bool {
    let slot = name
        .strip_prefix('v')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    !is_minified(name) && !slot
}

struct Collector<'s> {
    file: &'s str,
    source: &'s str,
    line_starts: &'s [u32],
    entries: Vec<Entry>,
}

impl Collector<'_> {
    fn record(&mut self, name: &str, at: u32, span: Span, node: impl FnOnce(&mut Tokens<'_>)) {
        let mut tokens = Tokens {
            source: self.source,
            set: FxHashSet::default(),
        };
        node(&mut tokens);
        let mut tokens: Vec<String> = tokens.set.into_iter().collect();
        tokens.sort_unstable();
        let text = self
            .source
            .get(span.start as usize..span.end as usize)
            .unwrap_or_default();
        let line = u32::try_from(self.line_starts.partition_point(|&start| start <= at)).unwrap_or(u32::MAX);
        self.entries.push(Entry {
            site: Site {
                name: name.to_string(),
                file: self.file.to_string(),
                line,
            },
            text_hash: fnv(text.as_bytes()),
            tokens,
        });
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if func.is_declaration()
            && let Some(id) = &func.id
        {
            self.record(&id.name, id.span.start, func.span, |t| t.visit_function(func, flags));
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        if class.is_declaration()
            && let Some(id) = &class.id
        {
            self.record(&id.name, id.span.start, class.span, |t| t.visit_class(class));
        }
        walk::walk_class(self, class);
    }

    fn visit_variable_declarator(&mut self, declarator: &VariableDeclarator<'a>) {
        if let (Some(id), Some(init)) = (declarator.id.get_binding_identifier(), &declarator.init)
            && matches!(
                init.without_parentheses(),
                Expression::FunctionExpression(_)
                    | Expression::ArrowFunctionExpression(_)
                    | Expression::ClassExpression(_)
            )
        {
            self.record(&id.name, id.span.start, init.span(), |t| t.visit_expression(init));
        }
        walk::walk_variable_declarator(self, declarator);
    }
}

/// The tokens of one function: string and template text, property names, and
/// identifiers that are names.
struct Tokens<'s> {
    source: &'s str,
    set: FxHashSet<String>,
}

impl<'a> Visit<'a> for Tokens<'_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        let token = match kind {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {
                let span = kind.span();
                let text = self
                    .source
                    .get(span.start as usize..span.end as usize)
                    .unwrap_or_default();
                (text.len() > 2).then(|| format!("s:{text}"))
            }
            AstKind::IdentifierName(it) => Some(format!("p:{}", it.name)),
            AstKind::IdentifierReference(it) if is_stable_name(&it.name) => Some(format!("i:{}", it.name)),
            _ => None,
        };
        if let Some(token) = token {
            self.set.insert(token);
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Shared tokens / all tokens of two sorted token lists.
fn jaccard(a: &[u32], b: &[u32]) -> f64 {
    let (mut i, mut j, mut shared) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = a.len() + b.len() - shared;
    if union == 0 {
        return 1.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let score = shared as f64 / union as f64;
    score
}

/// The pairs of an unmatched old and an unmatched new function whose token
/// sets are at least `min_score` similar, best first (score, old, new). An
/// old function is found through the tokens it shares with a new one; tokens
/// that more than 64 unmatched old functions have are skipped.
fn content_candidates(
    old_tokens: &[Vec<u32>],
    new_tokens: &[Vec<u32>],
    old_done: &[bool],
    new_done: &[bool],
    min_tokens: usize,
    min_score: f64,
) -> Vec<(f64, usize, usize)> {
    let mut postings: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (i, tokens) in old_tokens.iter().enumerate() {
        if !old_done[i] && tokens.len() >= min_tokens {
            for &token in tokens {
                postings.entry(token).or_default().push(i);
            }
        }
    }
    postings.retain(|_, list| list.len() <= 64);
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (j, tokens) in new_tokens.iter().enumerate() {
        if new_done[j] || tokens.len() < min_tokens {
            continue;
        }
        let mut seen: FxHashSet<usize> = FxHashSet::default();
        for token in tokens {
            for &i in postings.get(token).into_iter().flatten() {
                if seen.insert(i) {
                    let score = jaccard(&old_tokens[i], tokens);
                    if score >= min_score {
                        candidates.push((score, i, j));
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then((a.1, a.2).cmp(&(b.1, b.2))));
    candidates
}

/// The best and second-best candidate of one function in the weak pass.
#[derive(Debug, Clone, Copy, Default)]
struct Top {
    /// Best score and the index of its candidate.
    best: Option<(f64, usize)>,
    /// The runner-up's score, if there is one.
    second: Option<f64>,
}

impl Top {
    fn observe(&mut self, score: f64, other: usize) {
        let best = self.best;
        match best {
            Some((best, _)) if score <= best => {
                if self.second.is_none_or(|second| score > second) {
                    self.second = Some(score);
                }
            }
            _ => {
                self.second = self.best.map(|(best, _)| best);
                self.best = Some((score, other));
            }
        }
    }

    /// Whether `other` is the best candidate and beats the runner-up (if any)
    /// by at least `margin`.
    fn decisive(self, other: usize, margin: f64) -> bool {
        match (self.best, self.second) {
            (Some((best, j)), Some(second)) => j == other && best - second >= margin,
            (Some((_, j)), None) => j == other,
            (None, _) => false,
        }
    }
}

fn match_functions(old: &[Entry], new: &[Entry], options: TreeDiffOptions) -> TreeDiff {
    let mut interner: FxHashMap<String, u32> = FxHashMap::default();
    let mut intern = |entries: &[Entry]| -> Vec<Vec<u32>> {
        entries
            .iter()
            .map(|entry| {
                let mut ids: Vec<u32> = entry
                    .tokens
                    .iter()
                    .map(|token| {
                        if let Some(&id) = interner.get(token.as_str()) {
                            return id;
                        }
                        let id = u32::try_from(interner.len()).unwrap_or(u32::MAX);
                        interner.insert(token.clone(), id);
                        id
                    })
                    .collect();
                ids.sort_unstable();
                ids
            })
            .collect()
    };
    let old_tokens = intern(old);
    let new_tokens = intern(new);
    let mut diff = TreeDiff::default();
    diff.summary.functions_old = old.len();
    diff.summary.functions_new = new.len();
    let mut old_done = vec![false; old.len()];
    let mut new_done = vec![false; new.len()];

    // 1. Same stable name: within each name, best content pairs first.
    let mut by_name: FxHashMap<&str, (Vec<usize>, Vec<usize>)> = FxHashMap::default();
    for (i, entry) in old.iter().enumerate().filter(|(_, e)| is_stable_name(&e.site.name)) {
        by_name.entry(&entry.site.name).or_default().0.push(i);
    }
    for (j, entry) in new.iter().enumerate().filter(|(_, e)| is_stable_name(&e.site.name)) {
        by_name.entry(&entry.site.name).or_default().1.push(j);
    }
    let mut named: Vec<(usize, usize, f64)> = Vec::new();
    for (olds, news) in by_name.values() {
        let mut candidates: Vec<(f64, bool, usize, usize)> = olds
            .iter()
            .flat_map(|&i| news.iter().map(move |&j| (i, j)))
            .map(|(i, j)| {
                (
                    jaccard(&old_tokens[i], &new_tokens[j]),
                    old[i].site.file == new[j].site.file,
                    i,
                    j,
                )
            })
            .collect();
        candidates.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then((a.2, a.3).cmp(&(b.2, b.3)))
        });
        for (score, _, i, j) in candidates {
            if !old_done[i] && !new_done[j] {
                old_done[i] = true;
                new_done[j] = true;
                named.push((i, j, score));
            }
        }
    }
    named.sort_unstable_by_key(|&(i, j, _)| (i, j));
    for (i, j, score) in named {
        let changed = old[i].text_hash != new[j].text_hash;
        let pair = Pair {
            old: old[i].site.clone(),
            new: new[j].site.clone(),
            score: if changed { score } else { 1.0 },
            changed,
        };
        if changed {
            diff.summary.changed += 1;
            diff.changed.push(pair.clone());
        } else {
            diff.summary.same += 1;
        }
        if pair.old.file != pair.new.file {
            diff.summary.moved += 1;
            diff.moved.push(pair);
        }
    }

    // 2. The rest by content.
    let candidates = content_candidates(
        &old_tokens,
        &new_tokens,
        &old_done,
        &new_done,
        options.min_tokens,
        options.min_score,
    );
    for (score, i, j) in candidates {
        if old_done[i] || new_done[j] {
            continue;
        }
        old_done[i] = true;
        new_done[j] = true;
        diff.renamed.push(Pair {
            old: old[i].site.clone(),
            new: new[j].site.clone(),
            score,
            changed: old[i].text_hash != new[j].text_hash,
        });
    }
    diff.renamed.sort_by_key(|pair| Reverse((pair.score * 1e6) as i64));
    diff.summary.renamed = diff.renamed.len();

    // 3. Weak matches: what is still unmatched and changed a lot. A pair needs
    // at least `weak_min_score`, to be each other's best candidate, and to
    // beat both sides' runner-up by `weak_margin`; near-equal candidates are
    // not paired.
    let weak = content_candidates(
        &old_tokens,
        &new_tokens,
        &old_done,
        &new_done,
        options.min_tokens,
        options.weak_min_score,
    );
    let mut old_top = vec![Top::default(); old.len()];
    let mut new_top = vec![Top::default(); new.len()];
    for &(score, i, j) in &weak {
        old_top[i].observe(score, j);
        new_top[j].observe(score, i);
    }
    for &(score, i, j) in &weak {
        if old_top[i].decisive(j, options.weak_margin) && new_top[j].decisive(i, options.weak_margin) {
            old_done[i] = true;
            new_done[j] = true;
            diff.weak_renamed.push(Pair {
                old: old[i].site.clone(),
                new: new[j].site.clone(),
                score,
                changed: old[i].text_hash != new[j].text_hash,
            });
        }
    }
    diff.summary.weak_renamed = diff.weak_renamed.len();

    // 4. What is left: functions with a stable name only; minified and slot
    // names say nothing on their own.
    diff.removed = old
        .iter()
        .zip(&old_done)
        .filter(|(e, done)| !**done && is_stable_name(&e.site.name))
        .map(|(e, _)| e.site.clone())
        .collect();
    diff.added = new
        .iter()
        .zip(&new_done)
        .filter(|(e, done)| !**done && is_stable_name(&e.site.name))
        .map(|(e, _)| e.site.clone())
        .collect();
    diff.summary.removed = diff.removed.len();
    diff.summary.added = diff.added.len();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(files: &[(&str, &str)]) -> Vec<Entry> {
        files.iter().flat_map(|(file, code)| index_source(file, code)).collect()
    }

    #[test]
    fn pairs_by_name_then_by_content() {
        let old = entries(&[
            (
                "chunk-a.js",
                r#"function getSettings(){return load("settings.json").mode}function fn_11111111(x){return x.tengu_framed_osprey&&x.enabled&&x.policy}"#,
            ),
            ("chunk-b.js", "function keep(){return 1}"),
        ]);
        let new = entries(&[
            // keep moved to another file; the flag reader changed and so has a new shape name.
            (
                "chunk-c.js",
                "function keep(){return 1}function fn_22222222(x){return x.tengu_framed_osprey&&x.enabled&&x.policy&&x.extra}",
            ),
            (
                "chunk-d.js",
                r#"function getSettings(){return load("settings.json").mode??"auto"}function brandNew(){return "hello world"}"#,
            ),
        ]);
        let diff = match_functions(&old, &new, TreeDiffOptions::default());
        assert_eq!(diff.summary.same, 1, "{diff:?}");
        assert_eq!(diff.summary.changed, 1, "{diff:?}");
        assert_eq!(diff.summary.moved, 2, "{diff:?}");
        assert_eq!(diff.renamed.len(), 1, "{diff:?}");
        assert_eq!(diff.renamed[0].old.name, "fn_11111111");
        assert_eq!(diff.renamed[0].new.name, "fn_22222222");
        assert!(diff.renamed[0].changed);
        assert_eq!(
            diff.added.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["brandNew"]
        );
        assert!(diff.removed.is_empty(), "{diff:?}");
    }

    #[test]
    fn slot_and_minified_names_are_not_paired_by_name() {
        let old = entries(&[("a.js", "function Wd(){return 1}function v3(){return 2}")]);
        let new = entries(&[("a.js", r#"function Wd(){return "something else entirely"}"#)]);
        let diff = match_functions(&old, &new, TreeDiffOptions::default());
        assert_eq!(diff.summary.same + diff.summary.changed, 0, "{diff:?}");
    }

    #[test]
    fn weak_matches_need_a_mutual_clear_best() {
        // Shares 5 of 13 tokens: below the content threshold, above the weak one.
        let old = entries(&[(
            "a.js",
            "function fn_11111111(x){return x.a1+x.a2+x.a3+x.a4+x.a5+x.a6+x.a7+x.a8+x.a9}",
        )]);
        let new = entries(&[(
            "b.js",
            "function fn_22222222(x){return x.a1+x.a2+x.a3+x.a4+x.a5+x.b1+x.b2+x.b3+x.b4}",
        )]);
        let diff = match_functions(&old, &new, TreeDiffOptions::default());
        assert!(diff.renamed.is_empty(), "{diff:?}");
        assert_eq!(diff.weak_renamed.len(), 1, "{diff:?}");
        assert_eq!(diff.summary.weak_renamed, 1);
        assert_eq!(diff.weak_renamed[0].old.name, "fn_11111111");
        assert_eq!(diff.weak_renamed[0].new.name, "fn_22222222");
        assert!(diff.weak_renamed[0].changed);
        assert!(diff.removed.is_empty() && diff.added.is_empty(), "{diff:?}");
    }

    #[test]
    fn weak_matches_skip_near_equal_candidates() {
        // Both old functions share 5 of 13 tokens with the new one: a tie.
        let old = entries(&[(
            "a.js",
            "function fn_11111111(x){return x.a1+x.a2+x.a3+x.a4+x.a5+x.a6+x.a7+x.a8+x.a9}function fn_33333333(x){return x.a1+x.a2+x.a3+x.a4+x.a5+x.c1+x.c2+x.c3+x.c4}",
        )]);
        let new = entries(&[(
            "b.js",
            "function fn_22222222(x){return x.a1+x.a2+x.a3+x.a4+x.a5+x.b1+x.b2+x.b3+x.b4}",
        )]);
        let diff = match_functions(&old, &new, TreeDiffOptions::default());
        assert!(diff.weak_renamed.is_empty(), "{diff:?}");
        assert_eq!(diff.summary.weak_renamed, 0);
    }

    #[test]
    fn top_level_statements_pair_by_shape() {
        let old = entries(&[
            ("chunk-a.js", r#"console.log("tengu_x");"#),
            ("chunk-b.js", r#"console.log("tengu_y");"#),
        ]);
        let new = entries(&[
            ("chunk-c.js", r#"console.log("tengu_x");"#),
            ("chunk-d.js", r#"console.log("tengu_y", 1);"#),
        ]);
        let diff = match_functions(&old, &new, TreeDiffOptions::default());
        // The same statement in another file pairs by name; a changed one by content.
        assert_eq!(diff.summary.same, 1, "{diff:?}");
        assert_eq!(diff.moved.len(), 1, "{diff:?}");
        assert_eq!(diff.renamed.len(), 1, "{diff:?}");
        assert!(diff.renamed[0].changed, "{diff:?}");
        assert!(diff.renamed[0].old.name.starts_with("top_"), "{diff:?}");
        assert!(diff.renamed[0].new.name.starts_with("top_"), "{diff:?}");
    }

    #[test]
    fn lines_are_one_based() {
        let found = entries(&[("a.js", "\n\nfunction later(){}")]);
        assert_eq!(found[0].site.line, 3);
    }
}
