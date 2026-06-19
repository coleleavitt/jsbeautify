use super::super::match_kind::FileType;
use super::analyze;

fn kinds(src: &str) -> Vec<String> {
    analyze(src, None, FileType::Js)
        .into_iter()
        .map(|m| m.match_kind)
        .collect()
}

#[test]
fn flags_location_hash_to_innerhtml() {
    let src = "var x = location.hash; el.innerHTML = x;";
    assert!(kinds(src).contains(&"dom_xss".to_string()), "{:?}", kinds(src));
}

#[test]
fn flags_direct_source_in_sink() {
    let src = "document.body.innerHTML = location.search;";
    assert!(kinds(src).contains(&"dom_xss".to_string()));
}

#[test]
fn flags_open_redirect_from_tainted_href() {
    // The BlackRock idpRedirect shape: href set from a value derived from input.
    let src = "var u = location.href; window.location.href = u;";
    assert!(kinds(src).contains(&"open_redirect".to_string()), "{:?}", kinds(src));
}

#[test]
fn flags_eval_of_tainted() {
    let src = "var c = location.hash; eval(c);";
    assert!(kinds(src).contains(&"dom_xss".to_string()));
}

#[test]
fn flags_ssrf_fetch_of_tainted() {
    let src = "var u = location.search; fetch(u);";
    assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
}

#[test]
fn flags_ssrf_axios_get() {
    let src = "var u = location.hash; axios.get(u);";
    assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
}

#[test]
fn flags_ssrf_new_url_request() {
    let src = "var u = document.URL; new Request(u);";
    assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
}

#[test]
fn ssrf_ignores_map_get() {
    // `map.get(tainted)` is a Map lookup, not an HTTP request — must not flag.
    let src = "var k = location.hash; myMap.get(k);";
    assert!(!kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
}

#[test]
fn flags_ssrf_interprocedural() {
    // Tainted URL flows through a wrapper into fetch in a different function.
    let src = "
        function load(url) { return fetch(url); }
        var u = location.search;
        load(u);
    ";
    assert!(kinds(src).contains(&"ssrf".to_string()), "{:?}", kinds(src));
}

#[test]
fn ignores_constant_innerhtml() {
    let src = "el.innerHTML = '<b>static</b>';";
    assert!(kinds(src).is_empty(), "constant sink should not flag: {:?}", kinds(src));
}

#[test]
fn ignores_untainted_variable() {
    let src = "var safe = 'literal'; el.innerHTML = safe;";
    assert!(kinds(src).is_empty(), "{:?}", kinds(src));
}

#[test]
fn propagates_through_concatenation() {
    let src = "var h = location.hash; var html = '<div>' + h + '</div>'; el.innerHTML = html;";
    assert!(kinds(src).contains(&"dom_xss".to_string()), "{:?}", kinds(src));
}

#[test]
fn interprocedural_source_in_caller_sink_in_callee() {
    // Source is in the caller; the sink lives in a separate function reached
    // via the tainted argument. Intra-procedural taint would miss this.
    let src = "
        function render(html) { document.body.innerHTML = html; }
        var u = location.hash;
        render(u);
    ";
    assert!(kinds(src).contains(&"dom_xss".to_string()), "{:?}", kinds(src));
}

#[test]
fn interprocedural_arrow_function() {
    let src = "
        const go = (target) => { window.location.href = target; };
        var dest = location.search;
        go(dest);
    ";
    assert!(kinds(src).contains(&"open_redirect".to_string()), "{:?}", kinds(src));
}

#[test]
fn interprocedural_does_not_taint_unrelated_param() {
    // Only the tainted-position parameter becomes tainted, not a sibling.
    let src = "
        function sink(safe, danger) { el.innerHTML = safe; }
        var t = location.hash;
        sink('literal', t);
    ";
    // `safe` (position 0) got a literal; only `danger` (position 1) is tainted,
    // and it never reaches the sink — so no finding.
    assert!(kinds(src).is_empty(), "{:?}", kinds(src));
}

#[test]
fn method_call_does_not_taint_standalone_function() {
    // This test demonstrates the bug: calling a method with a tainted argument
    // should NOT taint a standalone function with the same name.
    let src = "
        function render(html) {
            document.body.innerHTML = html;
        }

        const obj = {
            render: function(data) {
                console.log(data);
            }
        };

        var userInput = location.hash;
        obj.render(userInput);
    ";

    let result = kinds(src);
    // Should be empty — no dom_xss finding because standalone render() is never called
    assert!(
        result.is_empty(),
        "False positive: tainted method call triggered standalone function's sink: {result:?}"
    );
}
