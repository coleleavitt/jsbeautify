use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::super::{
    AnalysisLimits, AnalysisOutcome, LiteralFoldKind, NotAnalyzedReason, SymbolRole, TruncatedAnalysisReason, analyze,
    analyze_with_limits, extract_wasm,
};

const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const MINIMAL_WASM: &[u8] = b"\0asm\x01\0\0\0";

fn manual_decoder(body: &str, literal: &str) -> String {
    format!(
        r#"
        function manual64(input) {{
            const alphabet = "{ALPHABET}";
            const output = new Uint8Array(Math.ceil(input.length * 3 / 4));
            {body}
        }}
        const decoded = manual64("{literal}");
        "#
    )
}

fn canonical_body() -> &'static str {
    r#"
        for (let i = 0, o = 0; i < input.length; i += 4, o += 3) {
            const a = alphabet.indexOf(input[i]);
            const b = alphabet.indexOf(input[i + 1]);
            const c = alphabet.indexOf(input[i + 2]);
            const d = alphabet.indexOf(input[i + 3]);
            output[o] = (a << 2) | (b >> 4);
            output[o + 1] = ((b & 15) << 4) | (c >> 2);
            output[o + 2] = ((c & 3) << 6) | d;
        }
        return output;
    "#
}

fn assert_not_recognized(label: &str, source: &str) {
    let report = analyze(source);
    assert!(
        report.literal_folds.is_empty(),
        "{label} was folded as a decoder: {report:#?}"
    );
    assert!(
        !report
            .symbol_role_suggestions
            .iter()
            .any(|suggestion| suggestion.role == SymbolRole::Base64Decoder),
        "{label} received a decoder role: {report:#?}"
    );
    assert!(
        extract_wasm(source).is_empty(),
        "{label} enabled narrow Wasm extraction"
    );
}

#[test]
fn manual_decoder_accepts_only_the_canonical_dataflow() {
    let encoded = STANDARD.encode(MINIMAL_WASM);
    let source = manual_decoder(canonical_body(), &encoded);
    let report = analyze(&source);
    assert_eq!(report.literal_folds.len(), 1);
    assert_eq!(report.literal_folds[0].kind, LiteralFoldKind::Base64Bytes);
    assert_eq!(extract_wasm(&source)[0].bytes, MINIMAL_WASM);
}

#[test]
fn manual_decoder_rejects_wrong_arithmetic_even_when_all_operator_tokens_are_present() {
    let encoded = STANDARD.encode(MINIMAL_WASM);
    let cases = [
        (
            "wrong first shift",
            canonical_body().replace("(a << 2) | (b >> 4)", "(a << 3) | (b >> 4)"),
        ),
        ("wrong mask", canonical_body().replace("(b & 15)", "(b & 7)")),
        ("wrong join operator", canonical_body().replace("(c >> 2)", "(c << 2)")),
        (
            "unrelated operator inventory",
            r#"
                const decoy = (1 << 2) | (2 >> 4) | (3 & 15);
                output[0] = 0;
                output[1] = decoy;
                output[2] = 0;
                return output;
            "#
            .to_string(),
        ),
    ];
    for (label, body) in cases {
        assert_not_recognized(label, &manual_decoder(&body, &encoded));
    }
}

#[test]
fn manual_decoder_rejects_permuted_alphabets_wrong_indices_and_wrong_input() {
    let encoded = STANDARD.encode(MINIMAL_WASM);
    let permuted = manual_decoder(canonical_body(), &encoded).replace(
        ALPHABET,
        "BACDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    );
    assert_not_recognized("permuted alphabet", &permuted);

    let cases = [
        (
            "duplicate input index",
            canonical_body().replace("input[i + 2]", "input[i + 1]"),
        ),
        (
            "wrong output index",
            canonical_body().replace("output[o + 2]", "output[o + 3]"),
        ),
        (
            "unrelated input",
            canonical_body().replace("alphabet.indexOf(input[i])", "alphabet.indexOf(other[i])"),
        ),
    ];
    for (label, body) in cases {
        assert_not_recognized(label, &manual_decoder(&body, &encoded));
    }
}

#[test]
fn manual_decoder_rejects_aliasing_side_effects_and_a_different_returned_buffer() {
    let encoded = STANDARD.encode(MINIMAL_WASM);
    let cases = [
        (
            "aliased output",
            canonical_body()
                .replace("for (let i", "const alias = output; for (let i")
                .replace("output[o]", "alias[o]")
                .replace("output[o + 1]", "alias[o + 1]")
                .replace("output[o + 2]", "alias[o + 2]"),
        ),
        (
            "observable side effect",
            canonical_body().replace("return output;", "globalThis.decodeCount++; return output;"),
        ),
        (
            "different returned buffer",
            canonical_body().replace("return output;", "return new Uint8Array(output);"),
        ),
        (
            "input mutation",
            canonical_body().replace("return output;", "input.length = 0; return output;"),
        ),
        (
            "unresolved global assignment",
            canonical_body().replace("return output;", "leaked = output; return output;"),
        ),
    ];
    for (label, body) in cases {
        assert_not_recognized(label, &manual_decoder(&body, &encoded));
    }
}

#[test]
fn literal_padding_property_covers_every_remainder_and_binary_payloads() {
    for len in 0_usize..=64 {
        let bytes = (0..len)
            .map(|index| ((index * 73 + len * 19) & 0xff) as u8)
            .collect::<Vec<_>>();
        let padded = STANDARD.encode(&bytes);
        let spellings = [padded.clone(), padded.trim_end_matches('=').to_string()];
        for spelling in spellings {
            let source = format!("function decode(value){{return atob(value);}} decode(\"{spelling}\");");
            let report = analyze(&source);
            assert_eq!(report.literal_folds.len(), 1, "len={len}, literal={spelling:?}");
            let fold = &report.literal_folds[0];
            assert_eq!(fold.byte_len, bytes.len(), "len={len}, literal={spelling:?}");
            assert_eq!(fold.digest_md5, format!("{:x}", md5::compute(&bytes)));
        }
    }
}

#[test]
fn malformed_padding_table_is_rejected_without_partial_folds() {
    for literal in ["A", "YQ=", "Y===", "=YQ=", "YQ=A", "YQ==A", "YQ===", "===="] {
        let source = format!("function decode(value){{return atob(value);}} decode(\"{literal}\");");
        assert!(
            analyze(&source).literal_folds.is_empty(),
            "malformed literal unexpectedly folded: {literal:?}"
        );
    }
}

#[test]
fn aggregate_limits_are_inclusive_at_the_boundary_and_fail_closed_one_below() {
    let encoded = STANDARD.encode(MINIMAL_WASM);
    let source = format!("function decode(value){{return atob(value);}} decode(\"{encoded}\");");
    let boundary_cases = [
        (
            "source bytes",
            AnalysisLimits {
                max_source_bytes: source.len(),
                ..AnalysisLimits::default()
            },
        ),
        (
            "encoded bytes",
            AnalysisLimits {
                max_encoded_literal_bytes: encoded.len(),
                ..AnalysisLimits::default()
            },
        ),
        (
            "decoded bytes",
            AnalysisLimits {
                max_decoded_bytes: MINIMAL_WASM.len(),
                ..AnalysisLimits::default()
            },
        ),
        (
            "decoder count",
            AnalysisLimits {
                max_decoders: 1,
                ..AnalysisLimits::default()
            },
        ),
        (
            "module count",
            AnalysisLimits {
                max_modules: 1,
                ..AnalysisLimits::default()
            },
        ),
    ];
    for (label, limits) in boundary_cases {
        assert_eq!(
            analyze_with_limits(&source, limits).literal_folds.len(),
            1,
            "inclusive {label} boundary rejected"
        );
    }

    let below_cases = [
        (
            "source bytes",
            AnalysisLimits {
                max_source_bytes: source.len() - 1,
                ..AnalysisLimits::default()
            },
        ),
        (
            "encoded bytes",
            AnalysisLimits {
                max_encoded_literal_bytes: encoded.len() - 1,
                ..AnalysisLimits::default()
            },
        ),
        (
            "decoded bytes",
            AnalysisLimits {
                max_decoded_bytes: MINIMAL_WASM.len() - 1,
                ..AnalysisLimits::default()
            },
        ),
        (
            "decoder count",
            AnalysisLimits {
                max_decoders: 0,
                ..AnalysisLimits::default()
            },
        ),
        (
            "module count",
            AnalysisLimits {
                max_modules: 0,
                ..AnalysisLimits::default()
            },
        ),
    ];
    for (label, limits) in below_cases {
        let report = analyze_with_limits(&source, limits);
        assert!(
            report.wasm_modules.is_empty(),
            "one-below {label} limit retained a Wasm report"
        );
        let expected_outcome = if label == "source bytes" {
            matches!(
                report.outcome,
                AnalysisOutcome::NotAnalyzed {
                    reason: NotAnalyzedReason::SourceTooLarge { .. }
                }
            )
        } else {
            matches!(
                report.outcome,
                AnalysisOutcome::Truncated {
                    reason: TruncatedAnalysisReason::AggregateBudgetExhausted
                }
            )
        };
        assert!(
            expected_outcome,
            "one-below {label} limit had the wrong outcome: {report:#?}"
        );
    }
}

#[test]
fn analysis_is_static_and_does_not_execute_hostile_javascript() {
    let source = r#"
        throw new Error("executed top level");
        function detonate() { while (true) {} }
        function decode(value) { return atob(value); }
        Object.defineProperty(globalThis, "payload", { get() { detonate(); } });
        decode("aGk=");
        detonate();
    "#;
    let report = analyze(source);
    assert_eq!(report.literal_folds.len(), 1);
    assert_eq!(report.literal_folds[0].utf8_preview.as_deref(), Some("hi"));
    assert!(extract_wasm(source).is_empty());
}
