use oxc_allocator::Allocator;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

/// Builds a human-readable context window around the first parse error of a
/// failed checkpoint re-parse. The byte offset is read from the diagnostic's
/// first labelled span via the typed `LabeledSpan::offset()` accessor (a stable
/// public API), then snapped to char boundaries so a multi-byte codepoint near
/// the edge can never panic the slice. A diagnostic with no label (e.g. the
/// source-size guard) falls back to offset 0.
pub(super) fn checkpoint_error_context(label: &str, code: &str, errors: &[OxcDiagnostic]) -> String {
    let Some(first) = errors.first() else {
        return format!("[DEOBFUSCATE] {label}: <no errors>");
    };
    let offset: usize = first
        .labels
        .first()
        .map_or(0, oxc_diagnostics::LabeledSpan::offset)
        .min(code.len() as u32) as usize;

    // Snap the window to char boundaries (offset may land mid-token but should
    // be on a boundary; the ±window edges might not be).
    let mut lo = offset.saturating_sub(300);
    let mut hi = (offset + 300).min(code.len());
    while lo > 0 && !code.is_char_boundary(lo) {
        lo -= 1;
    }
    while hi < code.len() && !code.is_char_boundary(hi) {
        hi += 1;
    }

    format!(
        "[DEOBFUSCATE] {label}: re-parse error at byte {offset} (output len={len}):\n\
         --- context [{lo}..{hi}] ---\n{window}\n--- end context ---",
        len = code.len(),
        window = &code[lo..hi],
    )
}

/// Logs validity of generated output and returns whether it re-parses cleanly.
///
/// Returns `false` only on a *parse* error (the output is not valid JS);
/// semantic errors are logged but still return `true`, matching the
/// parse-only policy the arena-split checkpoints enforce on their own codegen.
pub(super) fn log_output_validity(label: &str, code: &str, source_type: SourceType) -> bool {
    let allocator = Allocator::default();
    let parse_result = Parser::new(&allocator, code, source_type).parse();
    if !parse_result.diagnostics.is_empty() {
        eprintln!(
            "[DEOBFUSCATE] {label}: GENERATED OUTPUT IS INVALID JAVASCRIPT ({} parse error(s))",
            parse_result.diagnostics.len()
        );
        for (idx, error) in parse_result.diagnostics.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] {label}: parse error {idx}: {error:?}");
        }
        return false;
    }

    let semantic_result = SemanticBuilder::new().build(&parse_result.program);
    if !semantic_result.diagnostics.is_empty() {
        eprintln!(
            "[DEOBFUSCATE] {label}: GENERATED OUTPUT HAS {} SEMANTIC ERROR(S)",
            semantic_result.diagnostics.len()
        );
        for (idx, error) in semantic_result.diagnostics.iter().enumerate().take(5) {
            eprintln!("[DEOBFUSCATE] {label}: semantic error {idx}: {error:?}");
        }
        return true;
    }

    eprintln!("[DEOBFUSCATE] {label}: Generated output re-parse succeeded");
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    use oxc_codegen::Codegen;

    /// Parse `src` and codegen it back to a string with the same `Codegen`
    /// configuration the deobfuscation checkpoints use.
    fn roundtrip(src: &str) -> String {
        let allocator = Allocator::default();
        let program = Parser::new(&allocator, src, SourceType::mjs()).parse().program;
        Codegen::new().build(&program).code
    }

    /// The arena-split checkpoints (Phases 12.6 / 15.6) codegen the program,
    /// free the arena, then re-parse the codegen output into a fresh arena. This
    /// is only a behavioural no-op if oxc's `codegen ∘ parse` is idempotent on
    /// our own output — i.e. re-parsing and re-codegen-ing yields byte-identical
    /// text. Because the oxc dependency tracks upstream `main`, a future bump
    /// could silently break that property (and therefore the "1-vs-2-checkpoint
    /// output is byte-identical" guarantee); this test pins it.
    #[test]
    fn codegen_is_idempotent_across_a_checkpoint_roundtrip() {
        // Constructs the checkpoints must preserve across a parse/codegen cycle:
        // hashbang, directives, exports, template literals, regex, comments.
        let samples = [
            "#!/usr/bin/env node\n\"use strict\";\nconst a = 1 + 2;\nexport { a };\n",
            "function f(x){ return x ? `a${x}b` : /re,gex/g.test(x); }\nf(1);\n",
            "export function g(){ let q = 1; return q; }\nexport const K = 2;\n",
            "class C { #p = 1; m(){ return this.#p; } }\nnew C().m();\n",
            "const o = { a: 1, b: 2, ['c']: 3 };\nfor (const k of Object.keys(o)) o[k]++;\n",
        ];

        for src in samples {
            let once = roundtrip(src);
            let twice = roundtrip(&once);
            assert_eq!(
                once, twice,
                "codegen is not idempotent across a checkpoint round-trip for source:\n{src}\n\
                 first:\n{once}\nsecond:\n{twice}"
            );
        }
    }

    /// `checkpoint_error_context` reads the byte offset from a real parse
    /// diagnostic's labelled span (typed `LabeledSpan::offset()`), so a genuine
    /// syntax error must yield a context window centred on the offending byte —
    /// not the fragile string-scrape fallback of 0.
    #[test]
    pub(super) fn checkpoint_error_context_points_at_the_real_error_offset() {
        // A class body with a stray token: parse fails at a known position.
        let bad = "const a = 1;\nclass C { @@@ }\nconst b = 2;\n";
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, bad, SourceType::mjs()).parse();
        assert!(!parsed.diagnostics.is_empty(), "sample should fail to parse");

        let expected = parsed.diagnostics[0]
            .labels
            .first()
            .map(oxc_diagnostics::LabeledSpan::offset)
            .expect("parse error should carry a labelled span");

        let ctx = checkpoint_error_context("TEST", bad, &parsed.diagnostics);
        assert!(
            ctx.contains(&format!("error at byte {expected}")),
            "context should report the diagnostic's real offset {expected}, got:\n{ctx}"
        );
        // The window around a small input is the whole input, including the
        // offending `@` token; confirm the slice did not panic and is non-empty.
        assert!(
            ctx.contains("class C"),
            "context window should include the error site: {ctx}"
        );
    }

    /// Empty `code` must not panic the boundary-snapping / slice logic.
    #[test]
    pub(super) fn checkpoint_error_context_handles_empty_code() {
        let allocator = Allocator::default();
        // Produce at least one diagnostic, then format against empty code.
        let parsed = Parser::new(&allocator, "(", SourceType::mjs()).parse();
        assert!(!parsed.diagnostics.is_empty());
        let ctx = checkpoint_error_context("TEST", "", &parsed.diagnostics);
        assert!(
            ctx.contains("output len=0"),
            "should clamp offset/window to empty code: {ctx}"
        );
    }
}
