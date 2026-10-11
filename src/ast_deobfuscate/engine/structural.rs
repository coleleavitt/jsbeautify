use super::{Allocator, AstDeobfuscator, BunHelperMap, Codegen, Parser, SourceType, mem_ck};

impl AstDeobfuscator {
    pub(super) fn run_structural_half(
        &mut self,
        code: &str,
        source_type: SourceType,
    ) -> (String, BunHelperMap, SourceType) {
        // ── Structural half (Phases 0–12.5) runs in its own arena that is freed
        // before the cosmetic half re-parses. The transform passes accumulate a
        // large volume of dead intermediate AST in oxc's bump arena (which never
        // frees mid-run); codegen-ing at the Phase 12.6 checkpoint, dropping this
        // arena, then re-parsing the (valid) output into a fresh arena roughly
        // halves peak RSS. `bun_helper_map` is the only state that must cross.
        let allocator = Allocator::default();

        let mut source_type = source_type;
        let mut parse_result = Parser::new(&allocator, code, source_type).parse();

        // oxc recovers from syntax errors and still yields a usable partial
        // AST. Rather than abort the whole deobfuscation (which left odd or
        // non-JS inputs — e.g. an HTTP response dump — with no output at
        // all), log the diagnostics and continue on the recovered program.
        // The final codegen + re-parse checkpoints below validate the
        // result, and an input that recovers to an empty program is an error.
        if !parse_result.diagnostics.is_empty() {
            let diagnostic_count = parse_result.diagnostics.len();
            let first_diagnostic = parse_result.diagnostics.first().map(ToString::to_string);
            if let Some((fallback_source_type, fallback_parse_result)) =
                Self::parse_with_source_type_fallback(&allocator, code, source_type)
            {
                eprintln!(
                    "[DEOBFUSCATE] {diagnostic_count} parse diagnostic(s) with {source_type:?}; retrying as {fallback_source_type:?} (first: {first_diagnostic:?})",
                );
                source_type = fallback_source_type;
                parse_result = fallback_parse_result;
            } else {
                eprintln!(
                    "[DEOBFUSCATE] {diagnostic_count} parse diagnostic(s); continuing on recovered AST (first: {first_diagnostic:?})",
                );
            }
        }

        let mut program = parse_result.program;

        self.run_structural_preamble(&allocator, &mut program);
        crate::ast_deobfuscate::webpack::rename_webpack_bindings(&allocator, &mut program);
        let bun_helper_map = self.run_bun_naming_phases(&allocator, &mut program);
        let angular_renamed = crate::ast_deobfuscate::angular::rename_angular_symbols(&allocator, &mut program);
        eprintln!("[DEOBFUSCATE] Phase 0.995: Angular class/action naming renamed {angular_renamed} identifiers");
        self.run_structural_mid(&allocator, &mut program);
        self.run_structural_late(&allocator, &mut program);

        // ── Phase 12.6: structural checkpoint. Codegen the program, then the
        // structural arena is freed at the closing brace below — discarding all
        // dead intermediate AST accumulated by Phases 0–12.5.
        mem_ck("Phase 12.6: structural checkpoint codegen");
        eprintln!("[DEOBFUSCATE] Phase 12.6: structural checkpoint (codegen + arena reset)");
        (Codegen::new().build(&program).code, bun_helper_map, source_type)
    }

    fn parse_with_source_type_fallback<'a>(
        allocator: &'a Allocator,
        code: &'a str,
        source_type: SourceType,
    ) -> Option<(SourceType, oxc_parser::ParserReturn<'a>)> {
        // CommonJS last: it is the only source type that accepts a top-level
        // `return`, and the caller re-parses every checkpoint with it.
        for candidate in [
            source_type.with_typescript(true),
            source_type.with_jsx(true),
            source_type.with_typescript(true).with_jsx(true),
            SourceType::cjs(),
        ] {
            if candidate == source_type {
                continue;
            }
            let parse_result = Parser::new(allocator, code, candidate).parse();
            if parse_result.diagnostics.is_empty() && !parse_result.panicked {
                return Some((candidate, parse_result));
            }
        }
        None
    }
}
