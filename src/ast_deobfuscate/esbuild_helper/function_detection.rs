use oxc_ast::ast::Function;

use super::{AnthropicModification, EsbuildHelperCollector, EsbuildHelperInfo, EsbuildHelperKind, SupportHelperKind};

macro_rules! detect_regular_helpers {
    ($var_name:expr, $func:expr, [$($label:literal => $kind:ident via $detector:ident),+ $(,)?]) => {{
        $(
            if EsbuildHelperCollector::$detector($func) {
                eprintln!("[ESBUILD] Detected {}: {}", $label, $var_name);
                return Some((EsbuildHelperKind::$kind, None));
            }
        )+
        None
    }};
}

impl EsbuildHelperCollector {
    pub(super) fn try_detect_function_helper(&mut self, var_name: &str, func: &Function<'_>, line: u32) {
        let kind = self
            .detect_support_helper(var_name, func)
            .or_else(|| detect_regular_helper(var_name, func));

        if let Some((helper_kind, modification)) = kind {
            let mut info = EsbuildHelperInfo::new(var_name.to_string(), helper_kind, line);
            if let Some(mod_kind) = modification {
                info = info.with_modification(mod_kind);
            }
            self.helpers.insert(var_name.to_string(), info);
        }
    }

    fn detect_support_helper(
        &mut self,
        var_name: &str,
        func: &Function<'_>,
    ) -> Option<(EsbuildHelperKind, Option<AnthropicModification>)> {
        if Self::is_getter_bind_function(func) {
            eprintln!("[ESBUILD] Detected Anthropic getter bind helper: {var_name}");
            self.support_helpers
                .insert(var_name.to_string(), SupportHelperKind::GetterBind);
            Some((
                EsbuildHelperKind::SupportHelper(SupportHelperKind::GetterBind),
                Some(AnthropicModification::GetterBind),
            ))
        } else if Self::is_setter_bind_function(func) {
            eprintln!("[ESBUILD] Detected Anthropic setter bind helper: {var_name}");
            self.support_helpers
                .insert(var_name.to_string(), SupportHelperKind::SetterBind);
            Some((
                EsbuildHelperKind::SupportHelper(SupportHelperKind::SetterBind),
                Some(AnthropicModification::SetterBind),
            ))
        } else {
            None
        }
    }
}

fn detect_regular_helper(
    var_name: &str,
    func: &Function<'_>,
) -> Option<(EsbuildHelperKind, Option<AnthropicModification>)> {
    detect_regular_helpers!(var_name, func, [
        "__importDefault" => ImportDefault via is_import_default_pattern,
        "__importStar" => ImportStar via is_import_star_pattern,
        "__createBinding" => CreateBinding via is_create_binding_pattern,
        "__exportStar" => ExportStar via is_export_star_pattern,
        "__awaiter" => Awaiter via is_awaiter_pattern,
        "__generator" => Generator via is_generator_pattern,
        "__asyncGenerator" => AsyncGenerator via is_async_generator_pattern,
        "__privateGet" => PrivateGet via is_private_get_pattern,
        "__privateSet" => PrivateSet via is_private_set_pattern,
        "__privateAdd" => PrivateAdd via is_private_add_pattern,
        "__privateIn" => PrivateIn via is_private_in_pattern,
        "__addDisposableResource" => AddDisposableResource via is_add_disposable_resource_pattern,
        "__disposeResources" => DisposeResources via is_dispose_resources_pattern,
        "__extends" => Extends via is_extends_pattern,
        "__rest" => Rest via is_rest_pattern,
        "__assign" => Assign via is_assign_pattern,
        "__spread/__spreadArray" => SpreadArray via is_spread_pattern,
        "__values" => Values via is_values_pattern,
        "__read" => Read via is_read_pattern,
        "__decorateClass" => DecorateClass via is_decorate_class_pattern,
        "__decorate" => Decorate via is_decorate_pattern,
        "__param" => Param via is_param_pattern,
        "__metadata" => Metadata via is_metadata_pattern,
        "__template/__makeTemplateObject" => Template via is_template_pattern,
        "__name" => Name via is_name_pattern,
        "__superGet" => SuperGet via is_super_get_pattern,
        "__superSet" => SuperSet via is_super_set_pattern,
        "__pow" => Pow via is_pow_pattern,
        "__propKey" => PropKey via is_prop_key_pattern,
        "__toBinary" => ToBinary via is_to_binary_pattern,
        "__earlyAccess" => EarlyAccess via is_early_access_pattern,
        "__async" => Async via is_async_pattern,
        "__privateMethod" => PrivateMethod via is_private_method_pattern,
        "__publicField" => PublicField via is_public_field_pattern,
        "__privateWrapper" => PrivateWrapper via is_private_wrapper_pattern,
        "__require" => Require via is_require_pattern,
        "__glob" => Glob via is_glob_pattern,
        "__restKey" => RestKey via is_rest_key_pattern,
        "__objRest" => ObjRest via is_obj_rest_pattern,
        "__spreadValues" => SpreadValues via is_spread_values_pattern,
        "__spreadProps" => SpreadProps via is_spread_props_pattern,
        "__await" => Await via is_await_pattern,
        "__asyncDelegator" => AsyncDelegator via is_async_delegator_pattern,
        "__asyncValues" => AsyncValues via is_async_values_pattern,
        "__yieldStar" => YieldStar via is_yield_star_pattern,
        "__forAwait" => ForAwait via is_for_await_pattern,
        "__superWrapper" => SuperWrapper via is_super_wrapper_pattern,
        "__makeTemplateObject" => MakeTemplateObject via is_make_template_object_pattern,
        "__setFunctionName" => SetFunctionName via is_set_function_name_pattern,
        "__esDecorate" => EsDecorate via is_es_decorate_pattern,
        "__decorateParam" => DecorateParam via is_decorate_param_pattern,
        "__decoratorStart" => DecoratorStart via is_decorator_start_pattern,
        "__decoratorMetadata" => DecoratorMetadata via is_decorator_metadata_pattern,
        "__runInitializers" => RunInitializers via is_run_initializers_pattern,
        "__decorateElement" => DecorateElement via is_decorate_element_pattern,
        "__copyProps" => CopyProps via is_copy_props_pattern,
        "__reExport" => ReExport via is_re_export_pattern,
    ])
}
