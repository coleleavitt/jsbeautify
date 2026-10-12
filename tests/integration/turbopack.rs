//! Turbopack (Next.js 16) chunks through `extract-modules`, `split-chunks`,
//! `split-sources` and `-d`; browser, Node.js and development formats
//! (fixtures from Turbopack's snapshot tests, see `tests/fixtures/turbopack/`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CHUNK: &str = r#";!function(){try{var e="undefined"!=typeof globalThis?globalThis:{};e._debugIds={}}catch(e){}}();
(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["object"==typeof document?document.currentScript:void 0,983,e=>{"use strict";var t=e.i(913777);function n(){return t.jsx}e.s(["Spinner",0,n]);let a=1;e.s(["inner",()=>a],784018)},11,12,(e,t,r)=>{t.exports=e.r(983)}]);
//# sourceMappingURL=chunk.js.map"#;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jsb-turbopack-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_chunk(dir: &Path) -> PathBuf {
    let path = dir.join("chunk.js");
    std::fs::write(&path, CHUNK).expect("write chunk");
    path
}

fn jsbeautify(args: &[&std::ffi::OsStr]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_jsbeautify"))
        .args(args)
        .output()
        .expect("run jsbeautify");
    assert!(output.status.success(), "jsbeautify failed: {output:?}");
    output
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn extract_modules_writes_one_file_per_factory_and_hoisted_module() {
    let dir = temp_dir("extract");
    let chunk = write_chunk(&dir);
    let modules = dir.join("modules");
    let graph = dir.join("graph.dot");
    let output = jsbeautify(&[
        "extract-modules".as_ref(),
        chunk.as_os_str(),
        "-o".as_ref(),
        modules.as_os_str(),
        "--graph".as_ref(),
        graph.as_os_str(),
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Found 2 module factories and 1 scope-hoisted modules"),
        "{stderr}"
    );

    let first = read(&modules.join("module_983.js"));
    assert!(
        first.starts_with(
            "// Turbopack module 983\n// exports: Spinner\n// scope-hoisted module 784018: inner\n// imports: 913777\n"
        ),
        "{first}"
    );
    assert!(
        first.contains(r#".push(["object"==typeof document?document.currentScript:void 0, 983, e=>{"use strict";"#),
        "{first}"
    );
    let aliased = read(&modules.join("module_11.js"));
    assert!(aliased.contains("(also registered as 12)"), "{aliased}");
    assert!(
        aliased.contains(", 11, 12, (e,t,r)=>{t.exports=e.r(983)}]);"),
        "{aliased}"
    );
    assert!(read(&modules.join("hoisted/module_784018.js")).contains("scope-hoisted into module 983 (module_983.js)"));
    assert!(read(&graph).contains("\"11\" -> \"983\";"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_chunks_writes_modules_and_chunk_map() {
    let dir = temp_dir("split-chunks");
    let chunk = write_chunk(&dir);
    let out = dir.join("chunks");
    let map = dir.join("map.json");
    let output = jsbeautify(&[
        "split-chunks".as_ref(),
        chunk.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--chunk-map".as_ref(),
        map.as_os_str(),
    ]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("// Total files: 3"));
    assert!(out.join("module_983.js").exists() && out.join("module_11.js").exists());
    let map: serde_json::Value = serde_json::from_str(&read(&map)).expect("chunk map is json");
    assert_eq!(
        map["registrations"][0]["modules"][1]["ids"],
        serde_json::json!(["11", "12"])
    );
    assert_eq!(
        map["registrations"][0]["modules"][0]["dependencies"],
        serde_json::json!(["913777"])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_sources_falls_back_to_turbopack_modules_when_the_map_is_missing() {
    let dir = temp_dir("split-sources");
    let chunk = write_chunk(&dir);
    let out = dir.join("out");
    let output = jsbeautify(&[
        "split-sources".as_ref(),
        chunk.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    for path in [
        "synthetic/turbopack/chunk.js",
        "synthetic/turbopack/module_983.js",
        "synthetic/turbopack/module_11.js",
        "synthetic/turbopack/hoisted/module_784018.js",
    ] {
        assert!(stdout.contains(path), "{path} missing: {stdout}");
    }
    let skeleton = read(&out.join("reversed/synthetic/turbopack/chunk.js"));
    assert!(
        skeleton.contains("983,/* turbopack module 983 */,11,12,/* turbopack module 11 */]);"),
        "{skeleton}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deobfuscate_names_turbopack_context_imports_and_exports() {
    let dir = temp_dir("deob");
    let chunk = write_chunk(&dir);
    let output = jsbeautify(&["-d".as_ref(), chunk.as_os_str()]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Phase 0.86: Turbopack check: detected 1 registration(s), 2 module factories"),
        "{stderr}"
    );
    assert!(
        stdout.contains("var module_913777 = __turbopack_context__.i(913777);"),
        "{stdout}"
    );
    assert!(stdout.contains("function Spinner()"), "{stdout}");
    assert!(stdout.contains("const inner = 1;"), "{stdout}");
    assert!(
        stdout.contains("(__turbopack_context__, module, exports) =>"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/turbopack")
        .join(name)
}

fn deobfuscate(name: &str) -> (String, String) {
    let output = jsbeautify(&["-d".as_ref(), fixture(name).as_os_str()]);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    for phase in ["19.6", "21.5"] {
        assert!(
            stderr.contains(&format!("Phase {phase}: Generated output re-parse succeeded")),
            "{name}: {stderr}"
        );
    }
    (String::from_utf8_lossy(&output.stdout).into_owned(), stderr)
}

#[test]
fn deobfuscate_names_development_browser_chunk_imports_by_path() {
    let (stdout, stderr) = deobfuscate("dev_browser_chunk.js");
    assert!(
        stderr.contains("Phase 0.86: Turbopack check: detected 1 registration(s), 2 module factories"),
        "{stderr}"
    );
    assert!(
        stdout.contains(r#"var module_foo_index_js = __turbopack_context__.i("[project]/turbopack/crates/turbopack-tests/tests/snapshot/basic/async_chunk/input/node_modules/foo/index.js [test] (ecmascript)");"#),
        "{stdout}"
    );
    assert!(stdout.contains("(0, module_foo_index_js.foo)(true);"), "{stdout}");
    assert!(!stdout.contains("__TURBOPACK__imported__module__"), "{stdout}");
}

#[test]
fn deobfuscate_recognizes_node_chunks_entries_chunk_lists_and_workers() {
    for (name, detected) in [
        (
            "dev_node_chunk.js",
            "detected 1 registration(s), 1 module factories, Node.js chunk",
        ),
        (
            "node_strict_minified.js",
            "detected 1 registration(s), 2 module factories, Node.js chunk",
        ),
        ("node_entry.js", "detected Node.js entry"),
        ("dev_chunk_list.js", "detected 1 chunk list(s)"),
        ("worker_entrypoint.js", "detected worker entrypoint"),
    ] {
        let (stdout, stderr) = deobfuscate(name);
        assert!(
            stderr.contains(&format!("Phase 0.86: Turbopack check: {detected}")),
            "{name}: {stderr}"
        );
        if name == "node_strict_minified.js" {
            assert_eq!(stdout.matches("(__turbopack_context__) =>").count(), 2, "{stdout}");
        }
    }
}

#[test]
fn extract_modules_reexports_node_chunk_factories() {
    let dir = temp_dir("node-extract");
    let modules = dir.join("modules");
    let output = jsbeautify(&[
        "extract-modules".as_ref(),
        fixture("node_strict_minified.js").as_os_str(),
        "-o".as_ref(),
        modules.as_os_str(),
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Found 2 module factories"), "{stderr}");
    let lone =
        read(&modules.join(
            "module_turbopack_crates_turbopack_tests_tests_snapshot_mangle_exports_single_export_input_lone_js.js",
        ));
    assert!(
        lone.contains("// exports: f\n\"use strict\";\nmodule.exports = [\"[project]/"),
        "{lone}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_chunks_maps_node_entry_chunks_and_modules() {
    let dir = temp_dir("node-entry");
    let map = dir.join("map.json");
    jsbeautify(&[
        "split-chunks".as_ref(),
        fixture("node_entry.js").as_os_str(),
        "-o".as_ref(),
        dir.join("chunks").as_os_str(),
        "--chunk-map".as_ref(),
        map.as_os_str(),
    ]);
    let map: serde_json::Value = serde_json::from_str(&read(&map)).expect("chunk map is json");
    let entry = &map["registrations"][0];
    assert_eq!(entry["format"], "node-entry");
    assert_eq!(entry["callee"], r#"require("./[turbopack]_runtime.js")"#);
    assert_eq!(
        entry["other_chunks"],
        serde_json::json!(["output/1jsg_tests_snapshot_runtime_default_build_runtime_input_index_16lz1oyv57e07.js"])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deobfuscate_names_next16_build_output() {
    for (name, detected, expected) in [
        (
            "next16_nested_server_chunk.js",
            "detected 1 registration(s), 6 module factories, Node.js chunk",
            "var module_82509 = __turbopack_context__.i(82509);",
        ),
        (
            "next16_edge_entry_chunk.js",
            "detected 1 registration(s), 14 module factories, 1 edge entr(ies)",
            "new Proxy(modProm, {\n            get(innerModProm, name) {",
        ),
        (
            "next16_require_context_chunk.js",
            "detected 1 registration(s), 13 module factories, Node.js chunk",
            "const requireContext_52812 = __turbopack_context__.f(__turbopack_context__.r(52812));",
        ),
        (
            "next16_merged_modules_chunk.js",
            "detected 1 registration(s), 9 module factories, Node.js chunk",
            "var module_22918 = __turbopack_context__.i(22918);",
        ),
        (
            "next16_dev_refresh_chunk.js",
            "detected 1 registration(s), 2 module factories",
            "__turbopack_context__.k.register(refreshReg_Heavy, \"Heavy\");",
        ),
    ] {
        let (stdout, stderr) = deobfuscate(name);
        assert!(
            stderr.contains(&format!("Phase 0.86: Turbopack check: {detected}")),
            "{name}: {stderr}"
        );
        assert!(stdout.contains(expected), "{name}: {stdout}");
    }
}

#[test]
fn extract_modules_reads_next16_maps_entries_and_merged_modules() {
    let dir = temp_dir("next16-extract");
    let modules = dir.join("modules");
    let graph = dir.join("graph.dot");
    jsbeautify(&[
        "extract-modules".as_ref(),
        fixture("next16_require_context_chunk.js").as_os_str(),
        "-o".as_ref(),
        modules.as_os_str(),
        "--graph".as_ref(),
        graph.as_os_str(),
    ]);
    let context = read(&modules.join("module_52812.js"));
    assert!(
        context.contains("// require.context: ./moon.js -> 3456, ./star.js -> 28417\n"),
        "{context}"
    );
    assert!(read(&graph).contains("\"52812\" -> \"3456\";"));

    let edge = dir.join("edge");
    jsbeautify(&[
        "extract-modules".as_ref(),
        fixture("next16_edge_entry_chunk.js").as_os_str(),
        "-o".as_ref(),
        edge.as_os_str(),
    ]);
    let wrapper = read(&edge.join("module_28517.js"));
    assert!(
        wrapper.contains("// edge entry: self._ENTRIES[\"middleware_app/api/edge/route\"]\n"),
        "{wrapper}"
    );

    let out = dir.join("sources");
    let output = jsbeautify(&[
        "split-sources".as_ref(),
        fixture("next16_merged_modules_chunk.js").as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("synthetic/turbopack/merged/module_next_internal_server_app_form_page_actions_js_locals.js"),
        "{stdout}"
    );
    let actions = read(&out.join("reversed/synthetic/turbopack/module_34864.js"));
    assert!(
        actions.contains("// re-exports from an imported namespace: N as 0084f83ad5e72b4dcd50c5400ff217144b97ba7888, "),
        "{actions}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
