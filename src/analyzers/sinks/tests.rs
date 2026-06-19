use super::super::match_kind::FileType;
use super::{analyze, kinds};

fn kinds_for(src: &str) -> Vec<String> {
    analyze(src, None, FileType::Js)
        .into_iter()
        .map(|m| m.match_kind)
        .collect()
}

#[test]
fn finds_inner_html_assignment() {
    assert!(kinds_for("el.innerHTML = userInput;").contains(&kinds::HTML_MANIPULATION.to_string()));
    assert!(kinds_for("foo.outerHTML = x;").contains(&kinds::HTML_MANIPULATION.to_string()));
}

#[test]
fn finds_insert_adjacent_html() {
    assert!(kinds_for("el.insertAdjacentHTML('beforeend', x);").contains(&kinds::HTML_MANIPULATION.to_string()));
}

#[test]
fn finds_document_write() {
    assert!(kinds_for("document.write(x);").contains(&kinds::HTML_MANIPULATION.to_string()));
}

#[test]
fn finds_location_href_assignment() {
    let k = kinds_for("location.href = userInput;");
    assert!(k.contains(&kinds::LOCATION_ASSIGNMENT.to_string()), "got {k:?}");
}

#[test]
fn finds_location_assign_call() {
    let k = kinds_for("location.assign('https://evil.com');");
    assert!(k.contains(&kinds::LOCATION_ASSIGNMENT.to_string()), "got {k:?}");
}

#[test]
fn finds_location_replace_call() {
    let k = kinds_for("window.location.replace('x');");
    assert!(k.contains(&kinds::LOCATION_ASSIGNMENT.to_string()), "got {k:?}");
}

#[test]
fn finds_postmessage_call() {
    let k = kinds_for("frame.postMessage(data, '*');");
    assert!(k.contains(&kinds::POST_MESSAGE.to_string()), "got {k:?}");
}

#[test]
fn finds_window_onmessage() {
    let k = kinds_for("window.onmessage = handler;");
    assert!(k.contains(&kinds::WINDOW_ONMESSAGE.to_string()), "got {k:?}");
}

#[test]
fn finds_addeventlistener_message() {
    let k = kinds_for("window.addEventListener('message', handler);");
    assert!(k.contains(&kinds::WINDOW_ONMESSAGE.to_string()), "got {k:?}");
}

#[test]
fn finds_dangerously_set_inner_html() {
    let src = r"const x = <div dangerouslySetInnerHTML={{__html: y}} />;";
    let k = kinds_for(src);
    assert!(k.contains(&kinds::DANGEROUSLY_SET_INNER_HTML.to_string()), "got {k:?}");
}

#[test]
fn finds_url_search_params() {
    let k = kinds_for("new URLSearchParams(window.location.search);");
    assert!(k.contains(&kinds::URL_SEARCH_PARAMS.to_string()), "got {k:?}");
}

#[test]
fn no_false_positive_on_unrelated() {
    let k = kinds_for("const x = 1; function foo() { return x + 2; }");
    assert!(k.is_empty(), "got {k:?}");
}
