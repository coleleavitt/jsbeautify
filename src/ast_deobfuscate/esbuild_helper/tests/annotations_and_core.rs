use super::*;

#[test]
fn test_detect_object_alias() {
    let code = r"
        var __defProp = Object.defineProperty;
        var __getOwnPropNames = Object.getOwnPropertyNames;
    ";
    let collector = run_collector(code);
    assert!(collector.object_aliases.contains_key("__defProp"));
    assert!(collector.object_aliases.contains_key("__getOwnPropNames"));
}

#[test]
fn test_detect_commonjs_helper() {
    let code = r"
        var B = (q, K) => () => (K || q((K = {exports: {}}).exports, K), K.exports);
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("B"));
    assert_eq!(collector.helpers.get("B").unwrap().kind, EsbuildHelperKind::CommonJs);
}

#[test]
fn test_detect_esm_helper() {
    let code = r"
        var L = (q, K) => () => (q && (K = q(q = 0)), K);
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("L"));
    assert_eq!(collector.helpers.get("L").unwrap().kind, EsbuildHelperKind::Esm);
}

#[test]
fn test_detect_export_helper() {
    let code = r"
        var G8 = (q, K) => {
            for (var _ in K) __defProp(q, _, {get: K[_], enumerable: true});
        };
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("G8"));
    assert_eq!(collector.helpers.get("G8").unwrap().kind, EsbuildHelperKind::Export);
}

#[test]
fn test_detect_identity_helper() {
    let code = r"
        var sz5 = (q) => q;
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("sz5"));
    assert_eq!(
        collector.helpers.get("sz5").unwrap().kind,
        EsbuildHelperKind::SupportHelper(SupportHelperKind::Identity)
    );
    assert!(collector.support_helpers.contains_key("sz5"));
}

#[test]
fn test_detect_getter_bind_helper() {
    let code = r"
        function MH7(q) {
            return this[q];
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("MH7"));
    assert_eq!(
        collector.helpers.get("MH7").unwrap().kind,
        EsbuildHelperKind::SupportHelper(SupportHelperKind::GetterBind)
    );
    assert!(collector.support_helpers.contains_key("MH7"));
}

#[test]
fn test_detect_setter_bind_helper() {
    let code = r"
        function tz5(q, K) {
            this[q] = K;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("tz5"));
    assert_eq!(
        collector.helpers.get("tz5").unwrap().kind,
        EsbuildHelperKind::SupportHelper(SupportHelperKind::SetterBind)
    );
    assert!(collector.support_helpers.contains_key("tz5"));
}

#[test]
fn test_detect_anthropic_export() {
    let code = r"
        var G8 = (q, K) => {
            for (var _ in K) HI6(q, _, {
                get: K[_],
                enumerable: true,
                configurable: true,
                set: tz5.bind(K, _)
            });
        };
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("G8"));
    let helper = collector.helpers.get("G8").unwrap();
    assert_eq!(helper.kind, EsbuildHelperKind::Export);
    assert!(helper.is_anthropic_modified());
    assert!(
        helper
            .anthropic_modifications
            .contains(&AnthropicModification::ConfigurableExport)
    );
}

#[test]
fn test_bundle_type_standard() {
    let code = r"
        var B = (q, K) => () => (K || q((K = {exports: {}}).exports, K), K.exports);
        var L = (q, K) => () => (q && (K = q(q = 0)), K);
    ";
    let collector = run_collector(code);
    assert_eq!(collector.get_bundle_type(), BundleType::Standard);
}

#[test]
fn test_bundle_type_anthropic() {
    let code = r"
        var sz5 = (q) => q;
        function MH7(q) {
            return this[q];
        }
    ";
    let collector = run_collector(code);
    assert_eq!(collector.get_bundle_type(), BundleType::Anthropic);
}

#[test]
fn test_annotation_header_generation() {
    let code = r"
        var B = (q, K) => () => (K || q((K = {exports: {}}).exports, K), K.exports);
        var sz5 = (q) => q;
        function MH7(q) {
            return this[q];
        }
    ";
    let collector = run_collector(code);
    let annotated = annotate_esbuild_modules(code, &collector);

    assert!(annotated.contains("ESBUILD BUNDLE ANALYSIS"));
    assert!(annotated.contains("ANTHROPIC-MODIFIED ESBUILD"));
    assert!(annotated.contains("Helper Mapping"));
    assert!(annotated.contains("__commonJS"));
    assert!(annotated.contains("identity (Anthropic)"));
    assert!(annotated.contains("getterBind (Anthropic)"));
}
