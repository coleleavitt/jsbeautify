#[cfg(test)]
use super::*;

#[test]
fn test_basic_element_decompile() {
    let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "div");
                g.ɵɵtext(1);
                g.ɵɵelementEnd();
            }
            if (2 & rf) {
                g.ɵɵadvance(1);
                g.ɵɵtextInterpolate(ctx.name);
            }
        "#;

    let result = decompile_template_from_source(source).unwrap();
    assert!(result.contains("<div>"));
    assert!(result.contains("</div>"));
}

#[test]
fn test_nested_elements() {
    let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "div");
                g.ɵɵelementStart(1, "span");
                g.ɵɵtext(2, "hello");
                g.ɵɵelementEnd();
                g.ɵɵelementEnd();
            }
        "#;

    let result = decompile_template_from_source(source).unwrap();
    assert!(result.contains("<div>"));
    assert!(result.contains("<span>"));
    assert!(result.contains("hello"));
    assert!(result.contains("</span>"));
    assert!(result.contains("</div>"));
}

#[test]
fn test_self_closing_element() {
    let source = r#"
            if (1 & rf) {
                g.ɵɵelement(0, "br");
                g.ɵɵelement(1, "input");
            }
        "#;

    let result = decompile_template_from_source(source).unwrap();
    assert!(result.contains("<br />"));
    assert!(result.contains("<input />"));
}

#[test]
fn test_event_listener() {
    let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "button");
                g.ɵɵlistener("click", function($event) { return ctx.onClick(); });
                g.ɵɵtext(1, "Click me");
                g.ɵɵelementEnd();
            }
        "#;

    let result = decompile_template_from_source(source).unwrap();
    assert!(result.contains("(click)"));
    assert!(result.contains("<button"));
}

#[test]
fn test_property_binding() {
    let source = r#"
            if (1 & rf) {
                g.ɵɵelementStart(0, "div");
                g.ɵɵelementEnd();
            }
            if (2 & rf) {
                g.ɵɵproperty("ngClass", ctx.classes);
            }
        "#;

    let result = decompile_template_from_source(source).unwrap();
    assert!(result.contains("<div>"));
}

#[test]
fn test_emit_pseudo_typescript() {
    let comp = DecompiledComponent {
        selector: "lms-app-root".to_string(),
        class_name: "LmsAppRootComponent".to_string(),
        template_html: "<div>\n  <span>Hello</span>\n</div>\n".to_string(),
        inputs: vec!["title".to_string(), "mode".to_string()],
        outputs: vec!["closed".to_string()],
        dependencies: vec!["router".to_string(), "store".to_string()],
        standalone: true,
        encapsulation: ViewEncapsulation::None,
        consts: Vec::new(),
    };

    let ts = emit_pseudo_typescript(&comp);
    assert!(ts.contains("@Component"));
    assert!(ts.contains("selector: 'lms-app-root'"));
    assert!(ts.contains("standalone: true"));
    assert!(ts.contains("ViewEncapsulation.None"));
    assert!(ts.contains("@Input() title"));
    assert!(ts.contains("@Output() closed"));
    assert!(ts.contains("private router"));
    assert!(ts.contains("export class LmsAppRootComponent"));
}
