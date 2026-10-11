use super::*;

#[test]
fn test_detect_decorate_class() {
    let code = r"
        function __decorateClass(decorators, target, key, kind) {
            var result = target;
            for (var i = decorators.length - 1; i >= 0; i--) {
                var decorator = decorators[i];
                if (decorator) result = decorator(result) || result;
            }
            return result;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__decorateClass"));
    assert_eq!(
        collector.helpers.get("__decorateClass").unwrap().kind,
        EsbuildHelperKind::DecorateClass
    );
}

#[test]
fn test_detect_run_initializers() {
    let code = r"
        function __runInitializers(thisArg, initializers, value) {
            for (var i = 0; i < initializers.length; i++) {
                value = initializers[i].call(thisArg, value);
            }
            return value;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__runInitializers"));
    assert_eq!(
        collector.helpers.get("__runInitializers").unwrap().kind,
        EsbuildHelperKind::RunInitializers
    );
}

#[test]
fn test_detect_decorator_start() {
    let code = r"
        function __decoratorStart() {
            return [,,];
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__decoratorStart"));
    assert_eq!(
        collector.helpers.get("__decoratorStart").unwrap().kind,
        EsbuildHelperKind::DecoratorStart
    );
}

#[test]
fn test_detect_decorate_element() {
    let code = r"
        function __decorateElement(array, flags, name, decorators) {
            switch (flags) {
                case 1: return array;
                case 2: return array;
                case 3: return array;
                default: return array;
            }
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__decorateElement"));
    assert_eq!(
        collector.helpers.get("__decorateElement").unwrap().kind,
        EsbuildHelperKind::DecorateElement
    );
}

#[test]
fn test_detect_set_function_name() {
    let code = r#"
        function __setFunctionName(target, name, prefix) {
            if (typeof name === "symbol") name = "[" + name.description + "]";
            return Object.defineProperty(target, "name", { value: name });
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__setFunctionName"));
    assert_eq!(
        collector.helpers.get("__setFunctionName").unwrap().kind,
        EsbuildHelperKind::SetFunctionName
    );
}
