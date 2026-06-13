//! Deobfuscator integration tests over the obfuscated fixtures.

use js_beautify_rs::AstDeobfuscator;

use crate::fixtures;

#[test]
fn deobfuscates_string_array_fixture() {
    let src = fixtures::obfuscated("simple_string_array.js");
    let mut deob = AstDeobfuscator::new();
    let out = deob.deobfuscate(&src).expect("deobfuscate string-array sample");
    assert!(!out.trim().is_empty());
}

#[test]
fn deobfuscator_never_panics_on_any_fixture() {
    for (name, src) in fixtures::all_obfuscated() {
        let mut deob = AstDeobfuscator::new();
        // We don't assert a specific transformation here — only that the
        // deobfuscator handles every sample shape without panicking and
        // returns a result either way.
        let _ = deob
            .deobfuscate(&src)
            .unwrap_or_else(|e| panic!("deobfuscate {name} returned hard error: {e}"));
    }
}
