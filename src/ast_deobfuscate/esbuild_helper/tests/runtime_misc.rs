use super::*;

#[test]
fn test_detect_to_binary() {
    let code = r"
        function __toBinary(data) {
            var decoded = atob(data);
            return decoded;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__toBinary"));
    assert_eq!(
        collector.helpers.get("__toBinary").unwrap().kind,
        EsbuildHelperKind::ToBinary
    );
}

#[test]
fn test_detect_super_get() {
    let code = r"
        function __superGet(cls, self, key) {
            return Reflect.get(cls, key, self);
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__superGet"));
    assert_eq!(
        collector.helpers.get("__superGet").unwrap().kind,
        EsbuildHelperKind::SuperGet
    );
}

#[test]
fn test_detect_super_set() {
    let code = r"
        function __superSet(cls, self, key, value) {
            return Reflect.set(cls, key, value, self);
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__superSet"));
    assert_eq!(
        collector.helpers.get("__superSet").unwrap().kind,
        EsbuildHelperKind::SuperSet
    );
}

#[test]
fn test_detect_public_field() {
    let code = r"
        function __publicField(obj, key, value) {
            obj[key] = value;
            return value;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__publicField"));
    assert_eq!(
        collector.helpers.get("__publicField").unwrap().kind,
        EsbuildHelperKind::PublicField
    );
}

#[test]
fn test_detect_private_method() {
    let code = r#"
        function __privateMethod(obj, member, method) {
            __accessCheck(obj, member, "access private method");
            return method.bind(obj);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__privateMethod"));
    assert_eq!(
        collector.helpers.get("__privateMethod").unwrap().kind,
        EsbuildHelperKind::PrivateMethod
    );
}

#[test]
fn test_detect_require() {
    let code = r"
        function __require(x) {
            return new Proxy({}, {
                get: function() { return require; }
            });
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__require"));
    assert_eq!(
        collector.helpers.get("__require").unwrap().kind,
        EsbuildHelperKind::Require
    );
}

#[test]
fn test_detect_glob() {
    let code = r"
        function __glob(map) {
            var modules = {};
            return modules;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__glob"));
    assert_eq!(collector.helpers.get("__glob").unwrap().kind, EsbuildHelperKind::Glob);
}
