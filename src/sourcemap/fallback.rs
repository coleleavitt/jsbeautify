use super::tree::{ReversedSource, write_reversed_sources};
use crate::ast_deobfuscate::bun::{Bundler, detect_bundle_runtime};
use crate::bun_extract::{ModuleType, split_bundle};
use regex::Regex;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

static FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:async\s+)?function\s*\*?\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*\(").expect("valid function regex")
});

static CLASS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bclass\s+([A-Za-z_$][A-Za-z0-9_$]*)\b").expect("valid class regex"));

pub(super) fn reconstruct_without_sourcemap(bundle_source: &str, output_dir: &Path) -> Result<Vec<String>, String> {
    write_reversed_sources(synthetic_sources(bundle_source), output_dir)
}

fn synthetic_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let lazy_sources = lazy_module_sources(bundle_source);
    if !lazy_sources.is_empty() {
        return lazy_sources;
    }

    let boundary_sources = named_boundary_sources(bundle_source);
    if !boundary_sources.is_empty() {
        return boundary_sources;
    }

    vec![ReversedSource {
        source_path: "synthetic/chunks/bundle.js".to_string(),
        content: bundle_source.to_string(),
    }]
}

/// Splits the lazy `__esm` / `__commonJS` module wrappers that esbuild and Bun
/// emit, under a folder named for the bundler the runtime helpers identify.
fn lazy_module_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let split = split_bundle(bundle_source);
    if split.modules.is_empty() {
        return Vec::new();
    }
    let root = match detect_bundle_runtime(bundle_source).bundler {
        Some(Bundler::Bun) => "synthetic/bun",
        Some(Bundler::Esbuild) => "synthetic/esbuild",
        None => "synthetic/modules",
    };

    let mut out = Vec::with_capacity(split.modules.len() + 2);
    let mut paths = BTreeMap::new();
    push_source(&mut out, &mut paths, format!("{root}/runtime.js"), split.runtime);

    for (module, code) in split.modules {
        let kind = match module.module_type {
            ModuleType::CommonJs => "commonjs",
            ModuleType::Esm => "esm",
            ModuleType::Runtime | ModuleType::Main => "chunks",
        };
        push_source(
            &mut out,
            &mut paths,
            format!("{root}/{kind}/{}.js", safe_component(&module.name)),
            code,
        );
    }

    push_source(&mut out, &mut paths, format!("{root}/entry.js"), split.main);
    out
}

#[derive(Clone, Copy)]
enum BoundaryKind {
    Class,
    Function,
}

struct Boundary {
    range: Range<usize>,
    name: String,
    kind: BoundaryKind,
}

fn named_boundary_sources(bundle_source: &str) -> Vec<ReversedSource> {
    let mut boundaries = Vec::new();
    collect_boundaries(bundle_source, &FUNCTION_RE, BoundaryKind::Function, &mut boundaries);
    collect_boundaries(bundle_source, &CLASS_RE, BoundaryKind::Class, &mut boundaries);
    boundaries.sort_by_key(|b| (b.range.start, b.range.end));

    let mut out = Vec::new();
    let mut paths = BTreeMap::new();
    let mut last_end = 0;
    for boundary in boundaries {
        if boundary.range.start < last_end {
            continue;
        }
        last_end = boundary.range.end;
        let dir = match boundary.kind {
            BoundaryKind::Class => "classes",
            BoundaryKind::Function => "functions",
        };
        push_source(
            &mut out,
            &mut paths,
            format!("synthetic/{dir}/{}.js", safe_component(&boundary.name)),
            bundle_source[boundary.range].trim().to_string(),
        );
    }
    out
}

fn collect_boundaries(source: &str, re: &Regex, kind: BoundaryKind, out: &mut Vec<Boundary>) {
    for caps in re.captures_iter(source) {
        let Some(mat) = caps.get(0) else {
            continue;
        };
        let Some(name) = caps.get(1) else {
            continue;
        };
        let start = semantic_start(mat.start(), mat.as_str());
        let Some(end) = find_braced_region_end(source, start) else {
            continue;
        };
        out.push(Boundary {
            range: start..end,
            name: name.as_str().to_string(),
            kind,
        });
    }
}

fn semantic_start(offset: usize, matched: &str) -> usize {
    let mut skip = 0;
    for byte in matched.bytes() {
        if byte.is_ascii_whitespace() || byte == b';' {
            skip += 1;
        } else {
            break;
        }
    }
    offset + skip
}

fn find_braced_region_end(source: &str, start: usize) -> Option<usize> {
    let open = source.get(start..)?.find('{')? + start;
    matching_brace_end(source, open)
}

#[derive(Clone, Copy)]
enum ScanState {
    Code,
    LineComment,
    BlockComment,
    RegexLiteral { in_class: bool },
    String(u8),
}

fn matching_brace_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open).copied()? != b'{' {
        return None;
    }

    let mut depth = 1usize;
    let mut state = ScanState::Code;
    let mut last_significant = None;
    let mut i = open + 1;
    while i < bytes.len() {
        let byte = bytes[i];
        match state {
            ScanState::Code => match byte {
                b'{' => {
                    depth += 1;
                    last_significant = Some(byte);
                }
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(i + 1);
                    }
                    last_significant = Some(byte);
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    state = ScanState::LineComment;
                    i += 1;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    state = ScanState::BlockComment;
                    i += 1;
                }
                b'/' if regex_literal_can_start_after(last_significant) => {
                    state = ScanState::RegexLiteral { in_class: false };
                }
                b'"' | b'\'' | b'`' => state = ScanState::String(byte),
                _ if !byte.is_ascii_whitespace() => last_significant = Some(byte),
                _ => {}
            },
            ScanState::LineComment => {
                if byte == b'\n' {
                    state = ScanState::Code;
                }
            }
            ScanState::BlockComment => {
                if byte == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    state = ScanState::Code;
                    i += 1;
                }
            }
            ScanState::RegexLiteral { mut in_class } => {
                if byte == b'\\' {
                    i += 1;
                } else if byte == b'[' {
                    in_class = true;
                    state = ScanState::RegexLiteral { in_class };
                } else if byte == b']' {
                    in_class = false;
                    state = ScanState::RegexLiteral { in_class };
                } else if byte == b'/' && !in_class {
                    state = ScanState::Code;
                    last_significant = Some(b'/');
                }
            }
            ScanState::String(quote) => {
                if byte == b'\\' {
                    i += 1;
                } else if byte == quote {
                    state = ScanState::Code;
                    last_significant = Some(quote);
                }
            }
        }
        i += 1;
    }
    None
}

const fn regex_literal_can_start_after(previous: Option<u8>) -> bool {
    matches!(
        previous,
        None | Some(
            b'(' | b'{'
                | b'['
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
                | b'^'
                | b'~'
                | b'<'
                | b'>'
        )
    )
}

fn push_source(
    out: &mut Vec<ReversedSource>,
    paths: &mut BTreeMap<String, usize>,
    source_path: String,
    content: String,
) {
    if content.trim().is_empty() {
        return;
    }
    out.push(ReversedSource {
        source_path: unique_path(source_path, paths),
        content,
    });
}

fn unique_path(path: String, paths: &mut BTreeMap<String, usize>) -> String {
    let count = paths.entry(path.clone()).or_default();
    *count += 1;
    if *count == 1 {
        return path;
    }

    let suffix = *count;
    if let Some((stem, ext)) = path.rsplit_once('.') {
        format!("{stem}_{suffix}.{ext}")
    } else {
        format!("{path}_{suffix}")
    }
}

fn safe_component(raw: &str) -> String {
    let safe: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = safe.trim_matches('_');
    if trimmed.is_empty() {
        "anonymous".to_string()
    } else {
        trimmed.to_string()
    }
}
