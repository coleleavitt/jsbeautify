use super::*;
use crate::ast_deobfuscate::{DeobfuscateState, StringArrayInliner, StringArrayRotation};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;

fn run_decoder_inline(code: &str) -> (String, DecoderInliner) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let ret = Parser::new(&allocator, code, source_type).parse();
    let mut program = ret.program;

    let mut rotation = StringArrayRotation::new();
    let state = DeobfuscateState::new();
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);

    oxc_traverse::traverse_mut_with_ctx(&mut rotation, &mut program, &mut ctx);

    let mut state = DeobfuscateState::new();
    rotation.finalize(&mut state);

    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    let mut ctx = oxc_traverse::ReusableTraverseCtx::new(state, scoping, &allocator);

    let mut inliner = StringArrayInliner::new();
    oxc_traverse::traverse_mut_with_ctx(&mut inliner, &mut program, &mut ctx);

    let mut decoder = DecoderInliner::new();
    oxc_traverse::traverse_mut_with_ctx(&mut decoder, &mut program, &mut ctx);

    let output = Codegen::new().build(&program).code;
    (output, decoder)
}

#[test]
fn test_simple_decoder() {
    let code = r#"
            var _0x1234 = ["hello", "world", "test"];
            function _0xdec(a) {
                return _0x1234[a];
            }
            console.log(_0xdec(0));
            console.log(_0xdec(1));
        "#;

    let (output, decoder) = run_decoder_inline(code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0, "Should have inlined decoder calls");
    assert!(output.contains("\"hello\""), "Should contain inlined 'hello'");
    assert!(output.contains("\"world\""), "Should contain inlined 'world'");
    assert!(!output.contains("_0xdec(0)"), "Should not contain decoder call");
}

#[test]
fn test_decoder_with_offset() {
    let code = r#"
            var _0xabcd = ["foo", "bar", "baz"];
            function _0xdecode(a) {
                return _0xabcd[a - 100];
            }
            console.log(_0xdecode(100));
            console.log(_0xdecode(101));
        "#;

    let (output, decoder) = run_decoder_inline(code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0);
    assert!(output.contains("\"foo\""), "Should inline with offset");
    assert!(output.contains("\"bar\""), "Should inline with offset");
}

#[test]
fn test_decoder_with_assignment_offset() {
    let code = r#"
            var _0x1234 = ["a", "b", "c"];
            function _0xdec(a) {
                a = a - 291;
                return _0x1234[a];
            }
            console.log(_0xdec(291));
            console.log(_0xdec(292));
        "#;

    let (output, decoder) = run_decoder_inline(code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0);
    assert!(output.contains("\"a\""));
    assert!(output.contains("\"b\""));
}

#[test]
fn test_non_constant_arg() {
    let code = r#"
            var _0x1234 = ["hello", "world"];
            function _0xdec(a) {
                return _0x1234[a];
            }
            var i = 0;
            console.log(_0xdec(i));
        "#;

    let (output, decoder) = run_decoder_inline(code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() == 0, "Should not inline variable arg");
    assert!(output.contains("_0xdec(i)"), "Should preserve variable arg call");
}

#[test]
fn test_decoder_with_rotated_array() {
    let code = r#"
            var _0x1111 = ["a", "b", "c"];
            (function(_0x2222, _0x3333) {
                var _0x4444 = function(_0x5555) {
                    while (--_0x5555) {
                        _0x2222.push(_0x2222.shift());
                    }
                };
                _0x4444(2);
            })(_0x1111, 0x123);
            function _0xdec(a) {
                return _0x1111[a];
            }
            console.log(_0xdec(0));
        "#;

    let (output, decoder) = run_decoder_inline(code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0);
    assert!(output.contains("\"c\""), "Should use rotated array");
}

#[test]
fn test_base64_decoder() {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    let plain = "hello world";
    let encoded = STANDARD.encode(plain.as_bytes());

    let code = format!(
        r#"
            var _0xstr = ["{encoded}"];
            function _0xdec(a) {{
                return atob(_0xstr[a]);
            }}
            console.log(_0xdec(0));
            "#
    );

    let (output, decoder) = run_decoder_inline(&code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0, "Should have decoded base64");
    assert!(
        output.contains(&format!("\"{plain}\"")),
        "Should contain decoded string"
    );
}

#[test]
fn test_xor_decoder() {
    let plain = "HELLO";
    let key = b"\x01";
    let encoded: String = plain
        .chars()
        .map(|c| {
            let xored = (c as u8) ^ key[0];
            char::from(xored)
        })
        .collect();

    let code = format!(
        r#"
            var _0xstr = ["{encoded}"];
            function _0xdec(a) {{
                return xorDecode(_0xstr[a], "\x01");
            }}
            console.log(_0xdec(0));
            "#
    );

    let (output, decoder) = run_decoder_inline(&code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0, "Should have decoded XOR");
    assert!(
        output.contains(&format!("\"{plain}\"")),
        "Should contain decoded string"
    );
}

#[test]
fn test_rc4_decoder() {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use rc4::consts::U8;
    use rc4::{Key, KeyInit, Rc4, StreamCipher};

    let plain = "secret";
    let key = b"password";
    let key_arr = Key::<U8>::from_slice(key);
    let mut cipher = Rc4::<_>::new(key_arr);
    let mut encoded = plain.as_bytes().to_vec();
    cipher.apply_keystream(&mut encoded);

    let encoded_b64 = STANDARD.encode(&encoded);

    let code = format!(
        r#"
            var _0xstr = ["{encoded_b64}"];
            function _0xdec(a) {{
                return rc4Decode(_0xstr[a], "password");
            }}
            console.log(_0xdec(0));
            "#
    );

    let (output, decoder) = run_decoder_inline(&code);
    eprintln!("Output:\n{output}");

    assert_eq!(decoder.detected_decoders.len(), 1);
    assert!(decoder.rewrites() > 0, "Should have decoded RC4 and base64");
    assert!(
        output.contains(&format!("\"{plain}\"")),
        "Should contain decoded string"
    );
}
