use jsbeautify::AstDeobfuscator;
use jsbeautify::tokenizer::Tokenizer;
use jsbeautify::webpack_module_extractor::ModuleExtractor;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::fixtures;

fn deobfuscate_webpack_fixture(name: &str) -> String {
    let src = fixtures::webpack(name);
    let out = AstDeobfuscator::new()
        .deobfuscate(&src)
        .expect("deobfuscate webpack fixture");
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &out, SourceType::cjs()).parse();
    assert!(
        parsed.diagnostics.is_empty(),
        "deobfuscated {name} must parse: {:?}",
        parsed.diagnostics
    );
    out
}

#[test]
fn names_real_webpack5_production_runtime() {
    let out = deobfuscate_webpack_fixture("webpack5_prod_main.js");
    assert!(out.contains("function __webpack_require__(moduleId)"), "{out}");
    assert!(out.contains("__webpack_modules__[moduleId]("), "{out}");
    assert!(out.contains("(module, exports, __webpack_require__)"), "{out}");
    assert!(out.contains("(module, exports)"), "{out}");
    assert!(out.contains("module.exports = {"), "{out}");
    assert!(out.contains("var module_784 = __webpack_require__(784)"), "{out}");
    assert!(
        out.contains("__webpack_require__.d = ("),
        "runtime property names stay: {out}"
    );
    assert!(out.contains("self.webpackChunkjsbwp"), "{out}");
}

#[test]
fn names_real_webpack5_production_chunk() {
    let out = deobfuscate_webpack_fixture("webpack5_prod_lazy.js");
    assert!(out.contains("(module, exports, __webpack_require__)"), "{out}");
    assert!(out.contains("function lazyValue()"), "{out}");
    assert!(out.contains("function compute("), "{out}");
    assert!(out.contains("lazyValue: () => lazyValue"), "{out}");
}

#[test]
fn extracts_legacy_jsonp_modules_when_require_param_is_third_argument() {
    let code = r#"webpackJsonp([0],{1:function(t,e,n){"use strict";var r=n(2);},2:function(t,e,n){e.x=1;}},[1]);"#;
    let mut tokenizer = Tokenizer::new(code);
    let tokens = tokenizer.tokenize().expect("tokenize legacy jsonp bundle");
    let mut extractor = ModuleExtractor::new();

    extractor.extract_modules(&tokens).expect("extract modules");
    extractor.extract_dependencies(&tokens).expect("extract dependencies");

    assert_eq!(extractor.module_count(), 2);
    let temp_root = std::env::temp_dir().join(format!("jsbeautify-webpack-jsonp-{}", std::process::id()));
    std::fs::create_dir_all(&temp_root).expect("create temp dir");
    let graph_path = temp_root.join("deps.dot");
    extractor
        .generate_dependency_graph(&graph_path)
        .expect("write dependency graph");
    let graph = std::fs::read_to_string(&graph_path).expect("read dependency graph");
    std::fs::remove_dir_all(&temp_root).ok();

    assert!(graph.contains("module_1 -> module_2"));
}

#[test]
fn extracts_bootstrap_array_modules_when_main_bundle_uses_function_array() {
    let code = r"!function(e){function t(n){return e[n].call(null,null,null,t)}}([function(e,t,n){n(1);},function(e,t){t.x=1;}]);";
    let mut tokenizer = Tokenizer::new(code);
    let tokens = tokenizer.tokenize().expect("tokenize bootstrap bundle");
    let mut extractor = ModuleExtractor::new();

    extractor.extract_modules(&tokens).expect("extract modules");
    extractor.extract_dependencies(&tokens).expect("extract dependencies");

    assert_eq!(extractor.module_count(), 2);
    let temp_root = std::env::temp_dir().join(format!("jsbeautify-webpack-array-{}", std::process::id()));
    std::fs::create_dir_all(&temp_root).expect("create temp dir");
    let graph_path = temp_root.join("deps.dot");
    extractor
        .generate_dependency_graph(&graph_path)
        .expect("write dependency graph");
    let graph = std::fs::read_to_string(&graph_path).expect("read dependency graph");
    std::fs::remove_dir_all(&temp_root).ok();

    assert!(graph.contains("module_0 -> module_1"));
}
