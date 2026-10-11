use super::{
    AstDeobfuscator, BunHelper, BunHelperMap, Codegen, EsbuildHelperCollector, EsbuildHelperKind, Program, Result,
    SemanticBaseline, SourceType, annotate_bun_modules, annotate_esbuild_modules, annotate_namespace_modules,
    annotate_ts_enums, annotate_webpack_modules, expand_arrow_sequences, log_output_validity, mem_ck,
};

impl AstDeobfuscator {
    pub(super) fn generate_final_output(
        &self,
        program: &Program<'_>,
        source_type: SourceType,
        esbuild_collector: &EsbuildHelperCollector,
        bun_helper_map: &BunHelperMap,
        baseline: SemanticBaseline,
    ) -> Result<String> {
        mem_ck("before codegen (peak structural)");
        eprintln!("[DEOBFUSCATE] Generating output code");
        let output = Codegen::new().build(program).code;
        eprintln!("[DEOBFUSCATE] Output generated, {} bytes", output.len());
        // A cleanly parsed program may legitimately deobfuscate to nothing; an
        // unparseable input that recovers to nothing has no result to report.
        if output.trim().is_empty() && baseline == SemanticBaseline::Unknown {
            return Err(crate::BeautifyError::BeautificationFailed(
                "deobfuscation produced empty output from a recovered AST".to_string(),
            ));
        }
        // Consistency with the arena-split checkpoints: an AST -> codegen boundary
        // that emits unparseable JS indicates a real defect in a prior pass, so
        // surface it rather than returning invalid output as success.
        if !log_output_validity("Phase 19.6", &output, source_type, baseline) {
            return Err(crate::BeautifyError::BeautificationFailed(
                "Phase 19.6 final codegen produced invalid JavaScript".to_string(),
            ));
        }

        if self.skip_annotations {
            eprintln!("[DEOBFUSCATE] Skipping module annotations (alignment mode)");
            return Ok(output);
        }

        eprintln!("[DEOBFUSCATE] Phase 20: Annotating modules");
        let output = annotate_webpack_modules(&output);
        let output = annotate_esbuild_modules(&output, esbuild_collector);

        let mut esm_helpers: Vec<String> = esbuild_collector
            .get_helpers()
            .iter()
            .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::Esm))
            .map(|(name, _)| name.clone())
            .collect();
        let mut cjs_helpers: Vec<String> = esbuild_collector
            .get_helpers()
            .iter()
            .filter(|(_, info)| matches!(info.kind, EsbuildHelperKind::CommonJs))
            .map(|(name, _)| name.clone())
            .collect();

        // Augment from Bun runtime collector if esbuild didn't find anything
        if esm_helpers.is_empty() {
            for (name, helper) in bun_helper_map {
                if matches!(helper, BunHelper::Esm) {
                    esm_helpers.push(name.clone());
                }
            }
        }
        if cjs_helpers.is_empty() {
            for (name, helper) in bun_helper_map {
                if matches!(helper, BunHelper::CommonJs) {
                    cjs_helpers.push(name.clone());
                }
            }
        }
        let output = annotate_bun_modules(&output, &esm_helpers, &cjs_helpers);

        eprintln!("[DEOBFUSCATE] Phase 21: Post-codegen annotations (enums, namespaces, arrows)");
        let (output, enum_count) = annotate_ts_enums(&output);
        let (output, ns_count) = annotate_namespace_modules(&output);
        let (output, arrow_count) = expand_arrow_sequences(&output);
        eprintln!(
            "[DEOBFUSCATE] Phase 21: {enum_count} enum annotations, {ns_count} namespace annotations, {arrow_count} arrow expansions"
        );
        let (output, template_count) =
            crate::ast_deobfuscate::angular::annotate_angular_templates(&output, source_type);
        eprintln!("[DEOBFUSCATE] Phase 21: {template_count} Angular template annotations");
        // The Phase 20/21 post-codegen transforms (module annotations, arrow
        // expansion) rewrite the output string, so the final result is held to
        // the same parse-validity bar as the AST checkpoints: invalid JS is a
        // real defect and is surfaced rather than returned as success.
        if !log_output_validity("Phase 21.5", &output, source_type, baseline) {
            return Err(crate::BeautifyError::BeautificationFailed(
                "Phase 21.5 post-annotation output is invalid JavaScript".to_string(),
            ));
        }

        Ok(output)
    }
}
