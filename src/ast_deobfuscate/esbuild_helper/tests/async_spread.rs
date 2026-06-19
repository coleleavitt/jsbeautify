use super::*;

// ========== NEW TESTS FOR NEWLY-WIRED DETECTION FUNCTIONS ==========

#[test]
fn test_detect_awaiter() {
    let code = r"
        function Ql7(O, A, $, w) {
            return new ($ || ($ = Promise))(function(H, J) {
                H();
            });
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("Ql7"));
    assert_eq!(collector.helpers.get("Ql7").unwrap().kind, EsbuildHelperKind::Awaiter);
}

#[test]
fn test_detect_async_generator() {
    let code = r"
        function __asyncGenerator(thisArg, _arguments, generator) {
            var i = {};
            i[Symbol.asyncIterator] = function() { return this; };
            return i;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__asyncGenerator"));
    assert_eq!(
        collector.helpers.get("__asyncGenerator").unwrap().kind,
        EsbuildHelperKind::AsyncGenerator
    );
}

#[test]
fn test_detect_await() {
    let code = r"
        function __await(v) {
            return this instanceof __await ? (this.v = v, this) : new __await(v);
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__await"));
    assert_eq!(collector.helpers.get("__await").unwrap().kind, EsbuildHelperKind::Await);
}

#[test]
fn test_detect_async_delegator() {
    let code = r"
        function __asyncDelegator(o) {
            var i = { next: null, throw: null };
            return i;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__asyncDelegator"));
    assert_eq!(
        collector.helpers.get("__asyncDelegator").unwrap().kind,
        EsbuildHelperKind::AsyncDelegator
    );
}

#[test]
fn test_detect_async_values() {
    let code = r#"
        function __asyncValues(o) {
            if (!Symbol.asyncIterator) throw new TypeError("Symbol.asyncIterator is not defined.");
            var m = o[Symbol.asyncIterator], i;
            return m ? m.call(o) : i;
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__asyncValues"));
    assert_eq!(
        collector.helpers.get("__asyncValues").unwrap().kind,
        EsbuildHelperKind::AsyncValues
    );
}

#[test]
fn test_detect_values() {
    let code = r"
        function __values(o) {
            var m = o && o[Symbol.iterator];
            return m ? m.call(o) : null;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__values"));
    assert_eq!(
        collector.helpers.get("__values").unwrap().kind,
        EsbuildHelperKind::Values
    );
}

#[test]
fn test_detect_read() {
    let code = r"
        function __read(o, n) {
            var ar = [];
            try { while (true) ar.push(o.next().value); }
            catch (e) {}
            return ar;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__read"));
    assert_eq!(collector.helpers.get("__read").unwrap().kind, EsbuildHelperKind::Read);
}

#[test]
fn test_detect_rest() {
    let code = r"
        function Il7(O, A) {
            var $ = {};
            for (var w in O)
                if (A.indexOf(w)) $[w] = O[w];
            return $;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("Il7"));
    assert_eq!(collector.helpers.get("Il7").unwrap().kind, EsbuildHelperKind::Rest);
}

#[test]
fn test_detect_spread() {
    let code = r"
        function __spread() {
            var ar = [];
            return ar.concat(arguments);
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__spread"));
    assert_eq!(
        collector.helpers.get("__spread").unwrap().kind,
        EsbuildHelperKind::SpreadArray
    );
}

#[test]
fn test_detect_assign() {
    let code = r"
        function __assign(t) {
            for (var p in arguments) t[p] = arguments[p];
            return t;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__assign"));
    assert_eq!(
        collector.helpers.get("__assign").unwrap().kind,
        EsbuildHelperKind::Assign
    );
}

#[test]
fn test_detect_template() {
    let code = r"
        function __template(cooked, raw) {
            cooked.raw = raw;
            return cooked;
        }
    ";
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__template"));
    assert_eq!(
        collector.helpers.get("__template").unwrap().kind,
        EsbuildHelperKind::Template
    );
}

#[test]
fn test_detect_name() {
    let code = r#"
        function __name(target, value) {
            return (Object.defineProperty(target, "name", { value: value }), target);
        }
    "#;
    let collector = run_collector(code);
    assert!(collector.helpers.contains_key("__name"));
    assert_eq!(collector.helpers.get("__name").unwrap().kind, EsbuildHelperKind::Name);
}
