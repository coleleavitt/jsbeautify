//! End-to-end beautifier tests against the public API.

use js_beautify_rs::{Options, beautify};

use crate::fixtures;

#[test]
fn beautifies_minified_object() {
    let src = fixtures::obfuscated("real_bundle_sample.js");
    let out = beautify(&src, &Options::default()).expect("beautify minified object");
    assert!(out.contains("enabled"), "expected property names preserved");
    assert!(!out.trim().is_empty());
}

#[test]
fn every_fixture_round_trips_without_panicking() {
    // The headline regression: large/odd inputs must always yield *some*
    // output, never an empty file or a panic.
    for (name, src) in fixtures::all_obfuscated() {
        let out = beautify(&src, &Options::default()).unwrap_or_else(|e| panic!("beautify {name} failed: {e}"));
        assert!(!out.trim().is_empty(), "{name}: produced empty output");
    }
}

#[test]
fn beautify_is_idempotent_on_clean_input() {
    let clean = "function add(a, b) {\n\treturn a + b;\n}\n";
    let once = beautify(clean, &Options::default()).expect("first pass");
    let twice = beautify(&once, &Options::default()).expect("second pass");
    assert_eq!(once, twice, "beautify should be stable on already-clean input");
}

#[test]
fn recovers_from_trailing_garbage() {
    // Truncated/garbage-tailed bundles (as seen from 1 MiB-capped captures)
    // must still beautify the valid prefix rather than returning nothing.
    let src = "function ok() { return 1; } ### leftover";
    let out = beautify(src, &Options::default()).expect("recover valid prefix");
    assert!(out.contains("function ok"));
}
