use super::*;
use oxc_span::SourceType;

fn indexed(files: &[(&str, &str)]) -> Vec<IndexedFile> {
    indexed_with(files, false)
}

fn indexed_with(files: &[(&str, &str)], keep_fn_names: bool) -> Vec<IndexedFile> {
    let no_helpers = FxHashMap::default();
    let mut indexed: Vec<IndexedFile> = files
        .iter()
        .map(|(path, code)| {
            let source_type = if code.contains("TURBOPACK") {
                SourceType::script()
            } else {
                SourceType::mjs()
            };
            IndexedFile {
                path: (*path).to_string(),
                facts: collect_facts(code, source_type, &no_helpers, keep_fn_names),
            }
        })
        .collect();
    let graph = Graph::new(&indexed);
    let helpers: Vec<_> = (0..indexed.len()).map(|file| graph.imported_helpers(file)).collect();
    for ((file, (_, code)), helpers) in indexed.iter_mut().zip(files).zip(helpers) {
        if !helpers.is_empty() {
            file.facts = collect_facts(code, SourceType::mjs(), &helpers, keep_fn_names);
        }
    }
    indexed
}

fn import_names(seeds: &ProjectSeeds) -> Vec<(&str, Option<&str>, Option<&str>)> {
    seeds
        .imports
        .iter()
        .map(|s| (s.imported.as_str(), s.name.as_deref(), s.helper.as_deref()))
        .collect()
}

const RUNTIME: &str = r#"var i=Object.defineProperty;var b=(r,t)=>{for(var e in t)i(r,e,{get:t[e],enumerable:!0})};class V{constructor(){this.name="ValidationError"}}var n={n:0};export{b as a,V as v,n as c};"#;

#[test]
fn imported_helpers_and_names_follow_the_binding() {
    let files = indexed(&[
        ("chunk-ABCDEFGH.js", RUNTIME),
        (
            "sub/entry.js",
            r#"import{a as r,v as e,c as t}from"../chunk-ABCDEFGH.js";export const x=[r,new e,t];"#,
        ),
    ]);
    let plan = plan_project(&files);
    assert_eq!(
        import_names(&plan.seeds[1]),
        vec![
            ("a", Some("__export"), Some("__export")),
            ("v", Some("ValidationError"), None),
            ("c", Some("chunk_c"), None),
        ]
    );
    let exports: Vec<_> = plan.seeds[0]
        .exports
        .iter()
        .map(|s| (s.exported.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(
        exports,
        vec![("a", "__export"), ("v", "ValidationError"), ("c", "chunk_c")]
    );
    assert_eq!(plan.stats.imported_helpers, 1);
    assert_eq!(plan.stats.resolved_imports, 3);
}

#[test]
fn imported_export_helper_names_the_exporting_chunk() {
    let files = indexed(&[
        ("rt.js", RUNTIME),
        (
            "util.js",
            r#"import{a as r}from"./rt.js";var i={};r(i,{capitalize:()=>p});function p(e){return e}export{p as a,i as b};"#,
        ),
        (
            "entry.js",
            r#"import{a as e,b as t}from"./util.js";export{t as util};console.log(e("x"));"#,
        ),
    ]);
    let plan = plan_project(&files);
    assert_eq!(
        import_names(&plan.seeds[2]),
        vec![("a", Some("capitalize"), None), ("b", Some("util"), None)]
    );
}

#[test]
fn every_runtime_helper_variant_is_seeded_across_chunks() {
    let files = indexed(&[
        ("rt.js", r#"var j=e=>typeof e=="symbol"?e:e+"";export{j as k};"#),
        ("entry.js", r#"import{k as t}from"./rt.js";console.log(t(1));"#),
    ]);
    let plan = plan_project(&files);
    assert_eq!(
        import_names(&plan.seeds[1]),
        vec![("k", Some("__restKey"), Some("__restKey"))]
    );
}

#[test]
fn keep_fn_names_gives_importers_the_kept_function_names() {
    let files = [
        (
            "lib.js",
            "function p(e){return e}var o=()=>1,n={ k:1 };class C{}export{p as a,o as b,n as c,C as d};",
        ),
        (
            "user.js",
            r#"import{a as e,b as t,c as r,d as s}from"./lib.js";console.log(e,t,r,s);"#,
        ),
    ];
    let kept_files = indexed_with(&files, true);
    let kept: Vec<&str> = kept_files[0].facts.kept_locals.iter().map(String::as_str).collect();
    assert_eq!(kept, vec!["C", "o", "p"]);
    let plan = plan_project(&kept_files);
    assert_eq!(
        import_names(&plan.seeds[1]),
        vec![
            ("a", Some("p"), None),
            ("b", Some("o"), None),
            ("c", Some("lib_c"), None),
            ("d", Some("C"), None),
        ]
    );
    let exports: Vec<_> = plan.seeds[0]
        .exports
        .iter()
        .map(|s| (s.exported.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(exports, vec![("c", "lib_c")], "kept locals get no export seed");
    let default_plan = plan_project(&indexed(&files));
    assert_eq!(import_names(&default_plan.seeds[1])[0], ("a", Some("lib_a"), None));
}

#[test]
fn short_exports_are_names_only_when_public() {
    let files = indexed(&[
        ("lib.js", "var o=1,p=2;export{o as h,p as cx};"),
        ("user.js", r#"import{cx as q}from"./lib.js";console.log(q);"#),
    ]);
    let plan = plan_project(&files);
    let exports: Vec<_> = plan.seeds[0]
        .exports
        .iter()
        .map(|s| (s.exported.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(exports, vec![("h", "h"), ("cx", "lib_cx")]);
    assert_eq!((plan.stats.public_exports, plan.stats.internal_exports), (1, 1));
}

#[test]
fn dynamic_imports_make_every_export_internal() {
    let files = indexed(&[
        ("lazy.js", "var o=1,p=2;export{o as h,p as render};"),
        ("main.js", r#"import("./lazy.js").then(m=>m.h);"#),
    ]);
    let plan = plan_project(&files);
    let exports: Vec<_> = plan.seeds[0]
        .exports
        .iter()
        .map(|s| (s.exported.as_str(), s.name.as_str()))
        .collect();
    // `m.h` may be read: `h` is a link between chunks, not a name.
    assert_eq!(exports, vec![("h", "lazy_h"), ("render", "render")]);
}

#[test]
fn colliding_names_get_one_suffix_in_every_file() {
    let files = indexed(&[
        (
            "lib.js",
            r#"class A{constructor(){this.name="Text"}}class B{constructor(){this.name="Text"}}export{A as a,B as b};"#,
        ),
        ("user.js", r#"import{a as x,b as y}from"./lib.js";console.log(x,y);"#),
    ]);
    let plan = plan_project(&files);
    assert_eq!(
        import_names(&plan.seeds[1]),
        vec![("a", Some("Text"), None), ("b", Some("Text$1"), None)]
    );
}

#[test]
fn turbopack_keys_are_named_by_other_chunks_reexports() {
    let factory = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c1.js",13477,e=>{const t=(0,e.i(1).f)("work-async-storage");e.s(["f",0,t])}]);"#;
    let reexport = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["c2.js",52482,e=>{e.S([96154,0,13477,"workAsyncStorage,f"])}]);"#;
    let files = indexed(&[("c1.js", factory), ("c2.js", reexport)]);
    let plan = plan_project(&files);
    assert_eq!(
        plan.seeds[0].turbopack_reexports,
        vec![TurbopackReexport {
            module: "13477".to_string(),
            key: "f".to_string(),
            name: "workAsyncStorage".to_string(),
        }]
    );
    assert_eq!(plan.stats.turbopack_named_cross_file, 1);
    let output = crate::beautifier::deobfuscate_with_project_seeds(
        factory,
        &crate::Options {
            deobfuscate: true,
            ..crate::Options::default()
        },
        plan.seeds[0].clone(),
    )
    .expect("deobfuscates");
    assert!(output.contains("const workAsyncStorage = "), "{output}");
}

#[test]
fn links_report_broken_imports_and_name_agreement() {
    let inputs = indexed(&[
        ("lib.js", "var o=1;export{o as a};"),
        ("user.js", r#"import{a as q}from"./lib.js";console.log(q);"#),
    ]);
    let goal = |_: &Path| ParseGoal::Module;
    let same = vec![
        ("lib.js".to_string(), "var lib_a=1;export{lib_a as a};".to_string()),
        (
            "user.js".to_string(),
            r#"import{a as lib_a}from"./lib.js";console.log(lib_a);"#.to_string(),
        ),
    ];
    let report = check_links(&inputs, &same, goal);
    assert_eq!((report.imports_checked, report.consistent_names), (1, 1));
    assert!(report.broken_imports.is_empty() && report.changed_exports.is_empty());
    let broken = vec![
        ("lib.js".to_string(), "var o=1;export{o as b};".to_string()),
        (
            "user.js".to_string(),
            r#"import{a as q}from"./lib.js";console.log(q);"#.to_string(),
        ),
    ];
    let report = check_links(&inputs, &broken, goal);
    assert_eq!(report.broken_imports.len(), 1);
    assert_eq!(report.changed_exports, vec!["lib.js".to_string()]);
}

#[test]
fn file_idents_drop_hashes_and_chunk_prefixes() {
    assert_eq!(
        file_ident("assets/index.eager-common-TBZjjQ1o.js", true),
        "indexEagerCommon"
    );
    assert_eq!(
        file_ident("chunk-agent-character-picker-oYndMu-P.js", true),
        "agentCharacterPicker"
    );
    assert_eq!(file_ident("chunk-SNUETOXO.js", true), "chunk");
    assert_eq!(file_ident("chunk-SNUETOXO.js", false), "chunkSNUETOXO");
    assert_eq!(file_ident("1abc.js", true), "chunk1abc");
}
