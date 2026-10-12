use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

use super::{annotate_angular_templates, rename_angular_symbols};

fn rename(code: &str) -> String {
    let allocator = Allocator::default();
    let mut program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
    rename_angular_symbols(&allocator, &mut program, false);
    let out = Codegen::new().build(&program).code;
    let reparsed = Parser::new(&allocator, &out, SourceType::mjs()).parse();
    assert!(reparsed.diagnostics.is_empty(), "renamed output must parse:\n{out}");
    let semantic = SemanticBuilder::new().build(&reparsed.program);
    assert!(
        semantic.diagnostics.is_empty(),
        "renamed output must be semantically valid:\n{out}"
    );
    out
}

fn annotate(code: &str) -> (String, usize) {
    let (out, count) = annotate_angular_templates(code, SourceType::mjs());
    let allocator = Allocator::default();
    let reparsed = Parser::new(&allocator, &out, SourceType::mjs()).parse();
    assert!(reparsed.diagnostics.is_empty(), "annotated output must parse:\n{out}");
    let without_comments: Vec<&str> = out.lines().filter(|l| !l.trim_start().starts_with("// ")).collect();
    let original: Vec<&str> = code.lines().collect();
    assert_eq!(without_comments, original, "annotation may only add comment lines");
    (out, count)
}

fn template_lines(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.trim_start().strip_prefix("// "))
        .filter(|l| !l.starts_with("Angular template"))
        .map(str::to_string)
        .collect()
}

#[test]
fn renames_esbuild_static_component_and_its_variable() {
    let out = rename(r#"var Oc = class e { static ɵcmp = Vt({ type: e, selectors: [["app-login"]] }); }; use(Oc);"#);
    assert!(out.contains("var AppLoginComponent = class AppLoginComponent"), "{out}");
    assert!(out.contains("type: AppLoginComponent"), "{out}");
    assert!(out.contains("use(AppLoginComponent)"), "{out}");
}

#[test]
fn shorthand_property_keeps_its_key() {
    let out = rename(r#"var a = class e { static ɵcmp = Vt({ type: e, selectors: [["app-x"]] }); }; use({ a });"#);
    assert!(out.contains("a: AppXComponent"), "{out}");
}

#[test]
fn renames_iife_wrapped_directive_pipe_and_class_selector() {
    let out = rename(
        r#"
        var lD = (() => { class e { static { this.ɵdir = he({ type: e, selectors: [["", "appHighlight", ""]] }); } } return e; })();
        let P = (() => { class P {} P.ɵpipe = Zo({ name: "currency", type: P }); return P; })();
        var Cd = class { static ɵcmp = Vt({ type: Cd, selectors: [["", 8, "card"]] }); };
        use(lD, P, Cd);
        "#,
    );
    assert!(out.contains("var AppHighlightDirective = (() =>"), "{out}");
    assert!(out.contains("class AppHighlightDirective {"), "{out}");
    assert!(out.contains("let CurrencyPipe = (() =>"), "{out}");
    assert!(out.contains("var CardComponent = class"), "{out}");
    assert!(
        out.contains("use(AppHighlightDirective, CurrencyPipe, CardComponent)"),
        "{out}"
    );
}

#[test]
fn renames_ngrx_action_creators() {
    let out = rename(
        r#"var EN = zf("[Auth] Login Success", hD()), IN = zf("[Auth API] Logout"), c = zf("[X] Go"), d = f("Login Success");
        c = 1; use(EN, IN, c, d);"#,
    );
    assert!(out.contains("use(loginSuccess, logout, c, d)"), "{out}");
}

#[test]
fn leaves_readable_and_selectorless_classes() {
    let code = r#"var LoginForm = class LoginForm { static ɵcmp = Vt({ type: LoginForm, selectors: [["app-login"]] }); };
        var a = class e { static ɵcmp = Vt({ type: e, selectors: [["ng-component"]] }); }; use(LoginForm, a);"#;
    let out = rename(code);
    assert!(out.contains("use(LoginForm, a)"), "{out}");
    assert!(!out.contains("Component"), "{out}");
}

#[test]
fn rename_does_not_capture_shadowing_binding() {
    let out = rename(
        r#"var a = class e { static ɵcmp = Vt({ type: e, selectors: [["app-login"]] }); };
        function g(AppLoginComponent) { return [a, AppLoginComponent]; }"#,
    );
    assert!(out.contains("return [AppLoginComponent$1, AppLoginComponent]"), "{out}");
}

#[test]
fn rename_does_not_capture_global_reference() {
    let out = rename(
        r#"var a = class e { static ɵcmp = Vt({ type: e, selectors: [["app-login"]] }); };
        function g() { return [a, AppLoginComponent]; }"#,
    );
    assert!(out.contains("return [a, AppLoginComponent]"), "{out}");
}

#[test]
fn rename_respects_eval_and_exports() {
    let out = rename(
        r#"var a = class e { static ɵcmp = Vt({ type: e, selectors: [["app-a"]] }); };
        export var b = class f { static ɵcmp = Vt({ type: f, selectors: [["app-b"]] }); };
        eval("a");"#,
    );
    assert!(out.contains("var a = class e"), "{out}");
    assert!(out.contains("export var b = class f"), "{out}");
}

#[test]
fn inner_class_keeps_name_when_its_body_reads_the_variable() {
    let out = rename(
        r#"var a = class e { static ɵcmp = Vt({ type: e, selectors: [["x-y"]] }); m() { return a; } }; use(a);"#,
    );
    assert!(out.contains("var XYComponent = class e"), "{out}");
    assert!(out.contains("return XYComponent;"), "{out}");
}

#[test]
fn rename_is_inert_without_angular_definitions() {
    let code = "var a = class e { static x = f({ type: e, selectors: [[\"app-a\"]] }); }; use(a);";
    let allocator = Allocator::default();
    let mut program = Parser::new(&allocator, code, SourceType::mjs()).parse().program;
    assert_eq!(rename_angular_symbols(&allocator, &mut program, false), 0);
}

#[test]
fn decompiles_unminified_ivy_instructions() {
    let code = r#"class C {}
C.ɵcmp = i0.ɵɵdefineComponent({
    type: C,
    selectors: [["app-x"]],
    consts: [["id", "main", 1, "a", "b", 2, "color", "red", 3, "title"]],
    template: function C_Template(rf, ctx) {
        if (rf & 1) {
            i0.ɵɵelementStart(0, "div", 0);
            i0.ɵɵtext(1);
            i0.ɵɵelement(2, "br");
            i0.ɵɵelementEnd();
        }
        if (rf & 2) {
            i0.ɵɵproperty("title", ctx.title);
            i0.ɵɵadvance();
            i0.ɵɵtextInterpolate1("Hi ", ctx.user.name, "!");
        }
    }
});"#;
    let (out, count) = annotate(code);
    assert_eq!(count, 1);
    assert_eq!(
        template_lines(&out),
        [
            r#"<div id="main" class="a b" style="color: red" [title]="title">"#,
            "  Hi {{ user.name }}!",
            "  <br />",
            "</div>",
        ]
    );
}

#[test]
fn decompiles_minified_control_flow() {
    let code = r#"function T1(e, t) { e & 1 && (q(0, "li"), _e(1), Z()), e & 2 && (Ie(), Aa(t.$implicit)); }
function T2(e, t) { e & 1 && (q(0, "p"), _e(1, "Yes"), Z()); }
function T3(e, t) { e & 1 && (q(0, "p"), _e(1, "No"), Z()); }
var a = class e {
    static ɵcmp = Vt({
        type: e,
        selectors: [["app-list"]],
        consts: [[3, "click"]],
        template: function(n, r) {
            if (n & 1 && (q(0, "ul", 0), Ge("click", function() { return r.pick(); }), Jo(1, T1, 2, 1, "li", null, Qt), Z(), ri(3, "upper"), _e(4), Ko(5, T2, 2, 0, "p")(6, T3, 2, 0, "p")), n & 2) {
                Ie(), ei(r.items), Ie(3), Bt("Total: ", Md(3, 1, r.total)), Ie(), Xo(r.ok ? 5 : 6);
            }
        }
    });
};"#;
    let (out, _) = annotate(code);
    assert_eq!(
        template_lines(&out),
        [
            r#"<ul (click)="pick()">"#,
            "  @for (item of items; track Qt) {",
            "    <li>{{ t.$implicit }}</li>",
            "  }",
            "</ul>",
            "Total: {{ total | upper }}",
            "@if (ok) {",
            "  <p>Yes</p>",
            "} @else {",
            "  <p>No</p>",
            "}",
        ]
    );
}

#[test]
fn comment_lines_cannot_break_out() {
    let code = "var a = class e {\n    static ɵcmp = Vt({\n        type: e,\n        template: function(n, r) {\n            n & 1 && (q(0, \"b\"), _e(1, \"x\\u2028y */ z\\nw\"), Z());\n        }\n    });\n};";
    let (out, count) = annotate(code);
    assert_eq!(count, 1);
    assert!(template_lines(&out).iter().any(|l| l.contains("x y */ z w")), "{out}");
}

#[test]
fn self_referencing_embedded_template_terminates() {
    let code = "function T(e, t) { e & 1 && (q(0, \"i\"), Sa(1, T, 1, 0), Z()); }\nvar a = class e {\n    static ɵcmp = Vt({\n        type: e,\n        template: T\n    });\n};";
    let (out, count) = annotate(code);
    assert_eq!(count, 1);
    assert!(out.contains("<!-- … -->"), "{out}");
}

#[test]
fn annotation_is_inert_without_components() {
    let code = "var a = { template: function(n, r) { n & 1 && q(0, \"div\"); } };";
    assert_eq!(annotate(code), (code.to_string(), 0));
}
