use super::*;

#[test]
fn test_variable_declarations() {
    let code = "var r=t(123),n=t(456),o=t(789);";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains("t(123)"));
    assert!(result.contains("t(456)"));
    assert!(result.contains("t(789)"));
}

#[test]
fn test_nested_blocks_indentation() {
    let code = "function test() { if (x) { return 42; } }";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    let lines: Vec<&str> = result.lines().collect();
    assert!(lines.len() > 1, "Should have multiple lines");
    assert!(
        lines.iter().any(|l| l.starts_with('\t') || l.starts_with("  ")),
        "Should have indentation"
    );
}

#[test]
fn test_arrow_functions() {
    let code = "const fn = (x) => { return x * 2; };";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains("=>"));
    assert!(result.contains("return x * 2;"));
}

#[test]
fn test_template_literals() {
    let code = "const msg = `Hello ${name}`;";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains("`Hello ${name}`"));
}

#[test]
fn test_array_literals() {
    let code = "var arr = [1, 2, 3];";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains('1'));
    assert!(result.contains('2'));
    assert!(result.contains('3'));
    assert!(result.contains('['));
    assert!(result.contains(']'));
}

#[test]
fn test_object_literals() {
    let code = "var obj = {a: 1, b: 2};";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains('{'));
    assert!(result.contains('}'));
}

#[test]
fn test_comments_preserved() {
    let code = "// comment\nfunction test() { /* block */ return 1; }";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains("// comment"));
    assert!(result.contains("/* block */"));
}

#[test]
fn test_operators() {
    let code = "var x = 1 + 2 * 3;";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains("1 + 2 * 3"));
}

#[test]
fn test_ternary_operator() {
    let code = "var x = true ? 1 : 2;";
    let options = Options::default();
    let result = beautify(code, &options).unwrap();

    assert!(result.contains('?'));
    assert!(result.contains(':'));
}

mod layout {
    use super::*;
    use crate::options::{BraceStyle, Eol};
    use oxc_codegen::CodegenOptions;
    use oxc_span::SourceType;

    fn minified(code: &str) -> String {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        assert!(
            parsed.diagnostics.is_empty(),
            "invalid output: {:?}\n{code}",
            parsed.diagnostics
        );
        Codegen::new()
            .with_options(CodegenOptions::minify())
            .build(&parsed.program)
            .code
            .replace("\r\n", "\n")
    }

    fn wild() -> Vec<Options> {
        let base = Options::default();
        vec![
            base.clone(),
            Options {
                indent_size: 2,
                indent_level: 2,
                eol: Eol::CrLf,
                end_with_newline: true,
                brace_style: BraceStyle::Expand,
                jslint_happy: true,
                space_after_named_function: true,
                ..base.clone()
            },
            Options {
                indent_with_tabs: true,
                eol: Eol::Cr,
                brace_style: BraceStyle::EndExpand,
                max_preserve_newlines: 0,
                ..base.clone()
            },
            Options {
                preserve_newlines: false,
                ..base
            },
        ]
    }

    /// Layout must only change whitespace between tokens: the output
    /// re-parses to the same program as the plain codegen output.
    fn assert_sound(code: &str) {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
        let expected = minified(&Codegen::new().build(&parsed.program).code);
        for options in wild() {
            let out = beautify(code, &options).unwrap();
            assert_eq!(minified(&out), expected, "{options:?}\n{out}");
        }
    }

    fn run(code: &str, options: &Options) -> String {
        beautify(code, options).unwrap()
    }

    #[test]
    fn template_and_string_content_is_never_reindented() {
        let code = "function f(){if(a){return `x\n\ty${`\n\t\tnested`}\n  ${function(){return 1}}\n`}}\n\n\nvar t = `\n\n\n${a}\n`;\nvar s = 'a\\\n\tb';";
        assert_sound(code);
        let out = run(
            code,
            &Options {
                indent_size: 2,
                indent_level: 3,
                ..Options::default()
            },
        );
        assert!(out.contains("`x\n\ty${`\n\t\tnested`}\n  ${"), "{out}");
        assert!(out.contains("var t = `\n\n\n${a}\n`;"), "{out}");
    }

    #[test]
    fn template_line_ending_at_backtick_is_not_indented() {
        let code = "{ x = `abc\n`; y = `\n${a}`; }";
        assert_sound(code);
        let out = run(code, &Options::default());
        assert!(out.contains("x = `abc\n`;"), "{out}");
        assert!(out.contains("y = `\n${a}`;"), "{out}");
    }

    #[test]
    fn eol_conversion_keeps_template_values() {
        let code = "a = `one\r\ntwo\rthree\nfour`;\r\nb();";
        assert_sound(code);
        let out = run(code, &Options::default());
        assert!(out.contains("a = `one\r\ntwo\r\nthree\r\nfour`;\r\nb();"), "{out:?}");
    }

    #[test]
    fn eol_auto_uses_first_line_break() {
        assert_eq!(Eol::Auto.resolve("a\r\nb\n"), "\r\n");
        assert_eq!(Eol::Auto.resolve("a\rb"), "\r");
        assert_eq!(Eol::Auto.resolve("ab"), "\n");
        assert_eq!(Eol::Auto.resolve("a\u{2028}b\nc"), "\n");
    }

    #[test]
    fn indentation_options() {
        let code = "if(a){if(b){c()}}";
        assert_eq!(
            run(code, &Options::default()),
            "if (a) {\n    if (b) {\n        c();\n    }\n}"
        );
        let tabs = Options {
            indent_with_tabs: true,
            ..Options::default()
        };
        assert_eq!(run(code, &tabs), "if (a) {\n\tif (b) {\n\t\tc();\n\t}\n}");
        let level = Options {
            indent_size: 2,
            indent_level: 1,
            end_with_newline: true,
            ..Options::default()
        };
        assert_eq!(run(code, &level), "  if (a) {\n    if (b) {\n      c();\n    }\n  }\n");
        assert_eq!(run("  \tx()", &Options::default()), "  \tx();");
    }

    #[test]
    fn blank_lines_follow_preserve_options() {
        let code = "a();\n\n\n\nb();\n\n// note\n\nc();\nfunction f(){\n\n  d();\n\n}\n";
        assert_sound(code);
        assert_eq!(
            run(code, &Options::default()),
            "a();\n\n\n\nb();\n\n// note\n\nc();\n\nfunction f() {\n\n    d();\n\n}"
        );
        let capped = Options {
            max_preserve_newlines: 2,
            ..Options::default()
        };
        assert!(run(code, &capped).starts_with("a();\n\nb();"));
        let dropped = Options {
            preserve_newlines: false,
            ..Options::default()
        };
        assert_eq!(
            run(code, &dropped),
            "a();\nb();\n// note\nc();\n\nfunction f() {\n    d();\n}"
        );
    }

    #[test]
    fn blank_lines_map_columns_after_non_ascii_text() {
        let code = "var s = \"\u{1F600}é\"; var t = 1;\n\n\nx();\n\n/* ünï */ y();";
        assert_sound(code);
        let out = run(code, &Options::default());
        assert!(out.contains("var t = 1;\n\n\nx();"), "{out}");
    }

    #[test]
    fn no_leading_blank_lines_and_no_blank_inside_expressions() {
        let code = "\n\n\nfoo(a,\n\n\nb);\nvar o = {a:1,\n\n\nb:2};";
        assert_sound(code);
        let out = run(code, &Options::default());
        assert!(out.starts_with("foo(a, b);"), "{out}");
        assert!(out.contains("a: 1,\n\n\n    b: 2"), "{out}");
    }

    #[test]
    fn function_paren_spacing() {
        let code = "var f=function(){};var g=function*(){};function h(){}class A{m(){}get x(){return 1}[k](){}}var o={m(){},get y(){return 2}};";
        assert_sound(code);
        let plain = run(code, &Options::default());
        assert!(plain.contains("var g = function*() {};"), "{plain}");
        let anon = run(
            code,
            &Options {
                space_after_anon_function: true,
                ..Options::default()
            },
        );
        assert!(
            anon.contains("var f = function () {};") && anon.contains("function* () {}"),
            "{anon}"
        );
        assert!(anon.contains("function h() {}"), "{anon}");
        let named = run(
            code,
            &Options {
                space_after_named_function: true,
                ..Options::default()
            },
        );
        for expected in [
            "function h () {}",
            "m () {}",
            "get x () {",
            "[k]() {}",
            "get y() {",
            "var f = function() {};",
        ] {
            assert!(named.contains(expected), "{expected}: {named}");
        }
    }

    #[test]
    fn jslint_happy_aligns_cases_with_nested_switches() {
        let code = "switch(a){case 1:switch(b){case 2:x();break}break;default:y()}";
        assert_sound(code);
        let out = run(
            code,
            &Options {
                jslint_happy: true,
                ..Options::default()
            },
        );
        assert_eq!(
            out,
            "switch (a) {\ncase 1:\n    switch (b) {\n    case 2:\n        x();\n        break;\n    }\n    break;\ndefault:\n    y();\n}"
        );
    }

    #[test]
    fn brace_styles() {
        let code = "if(a){b()}else{c()}\ntry{d()}catch(e){f()}finally{g()}\ndo{h()}while(i)\nx=()=>{j()}";
        assert_sound(code);
        let expand = run(
            code,
            &Options {
                brace_style: BraceStyle::Expand,
                ..Options::default()
            },
        );
        assert_eq!(
            expand,
            "if (a)\n{\n    b();\n}\nelse\n{\n    c();\n}\ntry\n{\n    d();\n}\ncatch (e)\n{\n    f();\n}\nfinally\n{\n    g();\n}\ndo {\n    h();\n} while (i);\nx = () =>\n{\n    j();\n};"
        );
        let end_expand = run(
            code,
            &Options {
                brace_style: BraceStyle::EndExpand,
                ..Options::default()
            },
        );
        assert!(
            end_expand.starts_with("if (a) {\n    b();\n}\nelse {\n    c();\n}\ntry {"),
            "{end_expand}"
        );
    }

    #[test]
    #[allow(clippy::literal_string_with_formatting_args)]
    fn braces_and_declaration_lists_break_like_js_beautify() {
        let code = "var a={b:1},c,d=2,e;import {f} from 'g';export {f};var {h}=i;switch(j){case 1:k();}var o={m(){return {p:1}}};";
        assert_sound(code);
        let out = run(code, &Options::default());
        for expected in [
            "var a = {\n        b: 1\n    },\n    c, d = 2,\n    e;",
            "import {\n    f\n} from \"g\";",
            "export {\n    f\n};",
            "var {\n    h\n} = i;",
            "case 1:\n        k();",
            "var o = {\n    m() {\n        return {\n            p: 1\n        };\n    }\n};",
        ] {
            assert!(out.contains(expected), "{expected:?}\n{out}");
        }
    }

    #[test]
    fn comment_before_function_gets_no_extra_blank_line() {
        let code = "a();\n// about f\nfunction f(){}\nb();function g(){}";
        assert_eq!(
            run(code, &Options::default()),
            "a();\n// about f\nfunction f() {}\nb();\n\nfunction g() {}"
        );
    }

    #[test]
    fn asi_sensitive_inputs_survive_every_layout() {
        let cases = [
            "function f(){return\nx}",
            "a\n++b",
            "x\n(y)",
            "a\n[b]",
            "x\n`t`",
            "var a = b\n(function(){})()",
            "function g(){if(a){return\n{b:1}}else{throw c}}",
            "do{x()}while(y)\n[1].map(f)",
            "label:{break label}\nlet\nz = 1",
        ];
        for code in cases {
            for brace_style in [BraceStyle::Collapse, BraceStyle::Expand, BraceStyle::EndExpand] {
                for preserve_newlines in [true, false] {
                    let options = Options {
                        brace_style,
                        preserve_newlines,
                        jslint_happy: true,
                        space_after_named_function: true,
                        ..Options::default()
                    };
                    let out = beautify(code, &options).unwrap_or_else(|e| panic!("{code:?} {options:?}: {e}"));
                    let allocator = Allocator::default();
                    let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
                    assert_eq!(
                        minified(&out),
                        minified(&Codegen::new().build(&parsed.program).code),
                        "{code:?} {options:?}\n{out}"
                    );
                }
            }
        }
    }

    #[test]
    fn layout_rejects_text_that_changes_the_program() {
        let allocator = Allocator::default();
        let original = Parser::new(&allocator, "function f() { return x; }", SourceType::mjs()).parse();
        let expected = super::super::layout::minified(&original.program);
        let verify = |text: &str| super::super::layout::verify(&expected, true, text, SourceType::mjs());
        assert!(verify("function f() {\n    return x;\n}").is_ok());
        assert!(verify("function f() {\n    return\n    x;\n}").is_err());
        assert!(verify("function f() {\n    return x;\n").is_err());
    }

    #[test]
    fn deobfuscated_output_uses_the_layout() {
        let out = run(
            "function f(){if(a){return 1}}",
            &Options {
                deobfuscate: true,
                indent_size: 2,
                brace_style: BraceStyle::Expand,
                ..Options::default()
            },
        );
        assert!(!out.contains('\t'), "{out}");
        assert!(out.contains("\n{\n  if"), "{out}");
    }
}
