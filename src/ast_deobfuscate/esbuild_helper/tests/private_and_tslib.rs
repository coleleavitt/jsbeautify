use super::*;

#[test]
fn test_detect_import_default() {
    let code = r#"
        function __importDefault(mod) {
            return (mod && mod.__esModule) ? mod : { "default": mod };
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__importDefault"));
    assert_eq!(
        collector.helpers.get("__importDefault").unwrap().kind,
        EsbuildHelperKind::ImportDefault
    );
}

#[test]
fn test_detect_create_binding() {
    let code = r"
        function __createBinding(o, m, k, k2) {
            if (k2 === undefined) k2 = k;
            Object.defineProperty(o, k2, { enumerable: true, get: function() { return m[k]; } });
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__createBinding"));
    assert_eq!(
        collector.helpers.get("__createBinding").unwrap().kind,
        EsbuildHelperKind::CreateBinding
    );
}

#[test]
fn test_detect_export_star() {
    let code = r#"
        function __exportStar(m, exports) {
            for (var p in m) if (p !== "default") __createBinding(exports, m, p);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__exportStar"));
    assert_eq!(
        collector.helpers.get("__exportStar").unwrap().kind,
        EsbuildHelperKind::ExportStar
    );
}

#[test]
fn test_detect_generator() {
    let code = r"
        function __generator(thisArg, body) {
            var _ = { label: 0, sent: function() {}, trys: [], ops: [] };
            return _;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__generator"));
    assert_eq!(
        collector.helpers.get("__generator").unwrap().kind,
        EsbuildHelperKind::Generator
    );
}

#[test]
fn test_detect_private_add() {
    let code = r#"
        function __privateAdd(obj, state, value) {
            if (state.has(obj)) throw new TypeError("Cannot add same private member more than once");
            state.set(obj, value);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__privateAdd"));
    assert_eq!(
        collector.helpers.get("__privateAdd").unwrap().kind,
        EsbuildHelperKind::PrivateAdd
    );
}

#[test]
fn test_detect_private_get() {
    let code = r#"
        function __privateGet(receiver, state, kind, f) {
            if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a getter");
            return kind === "m" ? f : state.get(receiver);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__privateGet"));
    assert_eq!(
        collector.helpers.get("__privateGet").unwrap().kind,
        EsbuildHelperKind::PrivateGet
    );
}

#[test]
fn test_detect_private_set() {
    let code = r#"
        function __privateSet(receiver, state, value, kind, f) {
            if (kind === "m") throw new TypeError("Private method is not writable");
            state.set(receiver, value);
            return value;
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__privateSet"));
    assert_eq!(
        collector.helpers.get("__privateSet").unwrap().kind,
        EsbuildHelperKind::PrivateSet
    );
}

#[test]
fn test_detect_private_in() {
    let code = r#"
        function __privateIn(state, receiver) {
            return typeof state === "function" ? receiver === state : state.has(receiver);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__privateIn"));
    assert_eq!(
        collector.helpers.get("__privateIn").unwrap().kind,
        EsbuildHelperKind::PrivateIn
    );
}

#[test]
fn test_detect_dispose_resources() {
    let code = r#"
        function __disposeResources(env, suppressionError) {
            var fail = typeof SuppressedError === "function" ? SuppressedError : function(e, s) { return e; };
            return fail;
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__disposeResources"));
    assert_eq!(
        collector.helpers.get("__disposeResources").unwrap().kind,
        EsbuildHelperKind::DisposeResources
    );
}

#[test]
fn test_detect_extends() {
    let code = r"
        function __extends(d, b) {
            extendStatics(d, b);
            d.prototype = Object.create(b.prototype);
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__extends"));
    assert_eq!(
        collector.helpers.get("__extends").unwrap().kind,
        EsbuildHelperKind::Extends
    );
}

#[test]
fn test_detect_decorate() {
    let code = r"
        function __decorate(decorators, target, key, desc) {
            var c = arguments.length, r;
            for (var i = decorators.length - 1; i >= 0; i--) r = decorators[i](r);
            return r;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__decorate"));
    assert_eq!(
        collector.helpers.get("__decorate").unwrap().kind,
        EsbuildHelperKind::Decorate
    );
}

#[test]
fn test_detect_param() {
    let code = r"
        function __param(paramIndex, decorator) {
            return function(target, key) { decorator(target, key, paramIndex); };
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__param"));
    assert_eq!(collector.helpers.get("__param").unwrap().kind, EsbuildHelperKind::Param);
}

#[test]
fn test_detect_metadata() {
    let code = r#"
        function __metadata(metadataKey, metadataValue) {
            if (typeof Reflect === "object" && typeof Reflect.metadata === "function")
                return Reflect.metadata(metadataKey, metadataValue);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__metadata"));
    assert_eq!(
        collector.helpers.get("__metadata").unwrap().kind,
        EsbuildHelperKind::Metadata
    );
}

#[test]
fn test_detect_pow() {
    let code = r"
        function __pow(base, exp) {
            return Math.pow(base, exp);
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__pow"));
    assert_eq!(collector.helpers.get("__pow").unwrap().kind, EsbuildHelperKind::Pow);
}

#[test]
fn test_detect_prop_key() {
    let code = r#"
        function __propKey(x) {
            return typeof x === "symbol" ? x : "".concat(x);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__propKey"));
    assert_eq!(
        collector.helpers.get("__propKey").unwrap().kind,
        EsbuildHelperKind::PropKey
    );
}

#[test]
fn test_detect_early_access() {
    let code = r#"
        function __earlyAccess(name) {
            throw new ReferenceError("Cannot access '" + name + "' before initialization");
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__earlyAccess"));
    assert_eq!(
        collector.helpers.get("__earlyAccess").unwrap().kind,
        EsbuildHelperKind::EarlyAccess
    );
}
