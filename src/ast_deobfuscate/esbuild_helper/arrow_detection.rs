use oxc_ast::ast::ArrowFunctionExpression;

use super::{AnthropicModification, EsbuildHelperCollector, EsbuildHelperInfo, EsbuildHelperKind, SupportHelperKind};

impl EsbuildHelperCollector {
    pub(super) fn try_detect_helper(&mut self, var_name: &str, arrow: &ArrowFunctionExpression<'_>, line: u32) {
        if Self::is_identity_pattern(arrow) {
            eprintln!("[ESBUILD] Detected Anthropic identity helper: {var_name}");
            self.support_helpers
                .insert(var_name.to_string(), SupportHelperKind::Identity);
            self.helpers.insert(
                var_name.to_string(),
                EsbuildHelperInfo::new(
                    var_name.to_string(),
                    EsbuildHelperKind::SupportHelper(SupportHelperKind::Identity),
                    line,
                )
                .with_modification(AnthropicModification::IdentityFunction),
            );
            return;
        }

        let kind = if Self::is_commonjs_pattern(arrow) {
            EsbuildHelperKind::CommonJs
        } else if Self::is_esm_pattern(arrow) {
            EsbuildHelperKind::Esm
        } else if Self::is_export_pattern(arrow) {
            EsbuildHelperKind::Export
        } else if Self::is_toesm_pattern(arrow) {
            EsbuildHelperKind::ToEsm
        } else if Self::is_tocommonjs_pattern(arrow) {
            EsbuildHelperKind::ToCommonJs
        } else {
            return;
        };

        let mut info = EsbuildHelperInfo::new(var_name.to_string(), kind, line);

        if matches!(kind, EsbuildHelperKind::Export) && Self::has_anthropic_export_modifications(arrow) {
            eprintln!("[ESBUILD] Detected Anthropic __export modification: {var_name}");
            info = info.with_modification(AnthropicModification::ConfigurableExport);
        }

        eprintln!("[ESBUILD] Detected helper: {var_name} = {kind:?}");
        self.helpers.insert(var_name.to_string(), info);
    }
}
