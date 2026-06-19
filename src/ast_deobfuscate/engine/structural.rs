use super::{Allocator, AstDeobfuscator, BunHelperMap, Codegen, Parser, SourceType, mem_ck};

impl AstDeobfuscator {
    pub(super) fn run_structural_half(&mut self, code: &str, source_type: SourceType) -> (String, BunHelperMap) {
        // ── Structural half (Phases 0–12.5) runs in its own arena that is freed
        // before the cosmetic half re-parses. The transform passes accumulate a
        // large volume of dead intermediate AST in oxc's bump arena (which never
        // frees mid-run); codegen-ing at the Phase 12.6 checkpoint, dropping this
        // arena, then re-parsing the (valid) output into a fresh arena roughly
        // halves peak RSS. `bun_helper_map` is the only state that must cross.
        let allocator = Allocator::default();

        let parse_result = Parser::new(&allocator, code, source_type).parse();

        // oxc recovers from syntax errors and still yields a usable partial
        // AST. Rather than abort the whole deobfuscation (which left odd or
        // non-JS inputs — e.g. an HTTP response dump — with no output at
        // all), log the diagnostics and continue on the recovered program.
        // The final codegen + re-parse checkpoints below validate the
        // result, and the CLI/pipeline fall back to raw on empty output.
        if !parse_result.errors.is_empty() {
            eprintln!(
                "[DEOBFUSCATE] {} parse diagnostic(s); continuing on recovered AST (first: {:?})",
                parse_result.errors.len(),
                parse_result.errors.first().map(ToString::to_string),
            );
        }

        let mut program = parse_result.program;

        self.run_structural_preamble(&allocator, &mut program);
        let bun_helper_map = self.run_bun_naming_phases(&allocator, &mut program);
        self.run_structural_mid(&allocator, &mut program);
        self.run_structural_late(&allocator, &mut program);

        // ── Phase 12.6: structural checkpoint. Codegen the program, then the
        // structural arena is freed at the closing brace below — discarding all
        // dead intermediate AST accumulated by Phases 0–12.5.
        mem_ck("Phase 12.6: structural checkpoint codegen");
        eprintln!("[DEOBFUSCATE] Phase 12.6: structural checkpoint (codegen + arena reset)");
        (Codegen::new().build(&program).code, bun_helper_map)
    }
}
