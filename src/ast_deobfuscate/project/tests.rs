use super::*;
use crate::{AstDeobfuscator, ParseGoal};

fn deobfuscate(code: &str, seeds: ProjectSeeds) -> String {
    AstDeobfuscator::new()
        .with_parse_goal(ParseGoal::Module)
        .with_project_seeds(seeds)
        .deobfuscate(code)
        .expect("deobfuscation succeeds")
}

fn import_seed(source: &str, imported: &str, name: Option<&str>, helper: Option<&str>) -> ImportSeed {
    ImportSeed {
        source: source.to_string(),
        imported: imported.to_string(),
        name: name.map(str::to_string),
        helper: helper.map(str::to_string),
    }
}

#[test]
fn imported_export_helper_names_the_export_locals() {
    let code = r#"import{b as r}from"./chunk.js";var i={};r(i,{capitalize:()=>p,UTIL_VERSION:()=>o});function p(e){return e[0].toUpperCase()+e.slice(1)}var o="u1";console.log(p,o);export{i as c};"#;
    let alone = deobfuscate(code, ProjectSeeds::default());
    assert!(
        !alone.contains("capitalize("),
        "a single file cannot know the helper: {alone}"
    );
    let seeds = ProjectSeeds {
        imports: vec![import_seed("./chunk.js", "b", Some("__export"), Some("__export"))],
        ..ProjectSeeds::default()
    };
    let output = deobfuscate(code, seeds);
    assert!(output.contains("b as __export"), "{output}");
    assert!(output.contains("/* module "), "the helper is annotated: {output}");
    assert!(output.contains("function capitalize("), "{output}");
    assert!(output.contains("var UTIL_VERSION"), "{output}");
    assert!(output.contains(" as c"), "the export name stays: {output}");
}

#[test]
fn imported_commonjs_helper_names_the_module_handle() {
    let code = r#"import{a as u,c as m}from"./chunk.js";var i=u((h,s)=>{s.exports=1});var p=m(i());export{p as x};"#;
    let seeds = ProjectSeeds {
        imports: vec![
            import_seed("./chunk.js", "a", Some("__commonJS"), Some("__commonJS")),
            import_seed("./chunk.js", "c", Some("__toESM"), Some("__toESM")),
        ],
        ..ProjectSeeds::default()
    };
    let output = deobfuscate(code, seeds);
    assert!(output.contains("var require_1 = __commonJS("), "{output}");
    assert!(output.contains("__toESM(require_1())"), "{output}");
}

#[test]
fn seeds_rename_locals_but_keep_import_and_export_names() {
    let code = r#"import{a as e,b as t}from"./x.js";const n=e(t);export{n as q};"#;
    let seeds = ProjectSeeds {
        imports: vec![
            import_seed("./x.js", "a", Some("compute"), None),
            import_seed("./x.js", "b", Some("counter"), None),
        ],
        exports: vec![ExportSeed {
            exported: "q".to_string(),
            name: "result".to_string(),
        }],
        ..ProjectSeeds::default()
    };
    let output = deobfuscate(code, seeds);
    assert!(output.contains("a as compute"), "{output}");
    assert!(output.contains("b as counter"), "{output}");
    assert!(output.contains("const result = compute(counter)"), "{output}");
    assert!(output.contains("result as q"), "{output}");
}

#[test]
fn seeded_helpers_cover_every_helper_variant() {
    let allocator = oxc_allocator::Allocator::default();
    let code = r#"import{k as t,u as s}from"./rt.js";t(s);"#;
    let program = oxc_parser::Parser::new(&allocator, code, oxc_span::SourceType::mjs())
        .parse()
        .program;
    SemanticBuilder::new().build(&program);
    let seeds = ProjectSeeds {
        imports: vec![
            import_seed("./rt.js", "k", None, Some("__restKey")),
            import_seed("./rt.js", "u", None, Some("__callDispose")),
        ],
        ..ProjectSeeds::default()
    };
    let mut helpers: Vec<BunHelper> = seeded_helpers(&program, &seeds).into_values().collect();
    helpers.sort_unstable_by_key(BunHelper::canonical);
    assert_eq!(helpers, vec![BunHelper::CallDispose, BunHelper::RestKey]);
}

#[test]
fn keep_fn_names_keeps_seeded_function_locals_but_names_imports() {
    let code = r#"import{a as e}from"./x.js";function p(s){return e(s)}const n=e;export{p as q,n as r};"#;
    let seeds = ProjectSeeds {
        imports: vec![import_seed("./x.js", "a", Some("compute"), None)],
        exports: vec![
            ExportSeed {
                exported: "q".to_string(),
                name: "run".to_string(),
            },
            ExportSeed {
                exported: "r".to_string(),
                name: "alias".to_string(),
            },
        ],
        ..ProjectSeeds::default()
    };
    let output = AstDeobfuscator::new()
        .with_parse_goal(ParseGoal::Module)
        .with_keep_fn_names(true)
        .with_project_seeds(seeds)
        .deobfuscate(code)
        .expect("deobfuscation succeeds");
    assert!(output.contains("function p("), "p.name stays \"p\": {output}");
    assert!(output.contains("p as q"), "{output}");
    assert!(
        output.contains("a as compute"),
        "an import binding has no .name: {output}"
    );
    assert!(output.contains("const alias = compute"), "{output}");
}

#[test]
fn seeded_names_avoid_collisions_and_globals() {
    let code =
        r#"import{a as e}from"./x.js";function compute(){return e()}console.log(compute(),window);export{compute};"#;
    let seeds = ProjectSeeds {
        imports: vec![import_seed("./x.js", "a", Some("compute"), None)],
        ..ProjectSeeds::default()
    };
    let output = deobfuscate(code, seeds);
    assert!(output.contains("a as compute$1"), "{output}");
    let seeds = ProjectSeeds {
        imports: vec![import_seed("./x.js", "a", Some("window"), None)],
        ..ProjectSeeds::default()
    };
    let output = deobfuscate(code, seeds);
    assert!(output.contains("a as window$1"), "a global is never captured: {output}");
}

#[test]
fn eval_visible_bindings_keep_their_names() {
    let code = r#"import{a as e}from"./x.js";export function f(s){return eval(s)+e}"#;
    let seeds = ProjectSeeds {
        imports: vec![import_seed("./x.js", "a", Some("compute"), None)],
        ..ProjectSeeds::default()
    };
    let output = deobfuscate(code, seeds);
    assert!(output.contains("a as e"), "{output}");
}

#[test]
fn seed_json_round_trips() {
    let seeds = ProjectSeeds {
        imports: vec![import_seed("./x.js", "a", None, Some("__esm"))],
        turbopack_reexports: vec![TurbopackReexport {
            module: "12".to_string(),
            key: "a".to_string(),
            name: "Button".to_string(),
        }],
        ..ProjectSeeds::default()
    };
    let json = serde_json::to_string(&seeds).expect("serializes");
    assert!(!json.contains("\"exports\""), "{json}");
    assert_eq!(serde_json::from_str::<ProjectSeeds>(&json).expect("parses"), seeds);
    assert_eq!(BunHelper::from_canonical("__esm"), Some(BunHelper::Esm));
    assert_eq!(BunHelper::from_canonical("__callDispose"), Some(BunHelper::CallDispose));
    assert!(
        BunHelper::ALL
            .iter()
            .all(|h| BunHelper::from_canonical(h.canonical()) == Some(*h))
    );
}
