//! End-to-end Angular recovery on a trimmed real production build.

use js_beautify_rs::AstDeobfuscator;
use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::fixtures;

#[test]
fn recovers_angular_names_and_templates_from_production_build() {
    let path = fixtures::fixtures_dir().join("angular").join("main_trimmed.js");
    let src = std::fs::read_to_string(&path).expect("read Angular fixture");
    let out = AstDeobfuscator::new()
        .deobfuscate(&src)
        .expect("deobfuscate Angular sample");

    let allocator = Allocator::default();
    assert!(
        Parser::new(&allocator, &out, SourceType::mjs())
            .parse()
            .diagnostics
            .is_empty(),
        "output must parse"
    );
    for name in [
        "var AppLoginComponent = class AppLoginComponent",
        "var AppRootComponent = class AppRootComponent",
        "var AppTodoListComponent = class AppTodoListComponent",
        "var AppHighlightDirective = class AppHighlightDirective",
        "var ShoutPipe = class ShoutPipe",
        "class FormDirective",
        "var loginSuccess = ",
        "var logout = ",
    ] {
        assert!(out.contains(name), "missing `{name}`");
    }
    assert_eq!(out.matches("// Angular template (decompiled):").count(), 3);
    for line in [
        r#"//   <input id="email" type="email" name="email" [(ngModel)]="email" [placeholder]="hint" />"#,
        r#"// <h1 appHighlight class="title">Hello, {{ title() | shout }}</h1>"#,
        "// <p>Updated {{ now | date: \"short\" }}</p>",
        "// @defer {",
        "// } @placeholder {",
        "//   } @empty {",
        r#"// <app-login [hint]="hint" (loggedIn)="onLogin($event)"></app-login>"#,
    ] {
        assert!(out.contains(line), "missing template line `{line}`");
    }
}
