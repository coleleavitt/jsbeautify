mod adversarial;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::{
    AnalysisLimits, AnalysisOutcome, HandlerRole, JsPatternKind, LiteralFoldKind, NotAnalyzedReason,
    StructuredControlEdgeKind, SymbolRole, TruncatedAnalysisReason, VM_ANALYSIS_SCHEMA_VERSION, analyze,
    analyze_artifact_with_limits, analyze_with_limits, extract_wasm,
};

#[test]
fn detects_domain_shaped_vm_patterns() {
    let pool = (0..70)
        .map(|index| format!("\"literal_{index}\""))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r"
        const pool = [{pool}];
        const descriptors = [{{op: 1}}, {{op: 2}}, {{op: 3}}, {{op: 4}}];
        const handlers = [function(){{}}, x => x, function(){{}}, x => x];
        function run(frame) {{ frame.stack.push(frame.locals[0]); }}
        const module = new WebAssembly.Module(bytes);
        const instance = new WebAssembly.Instance(module, imports);
        const blob = new Blob([workerSource]);
        const worker = new Worker(URL.createObjectURL(blob));
        "
    );

    let report = analyze(&source);
    for kind in [
        JsPatternKind::LargeLiteralPool,
        JsPatternKind::DescriptorObjectArray,
        JsPatternKind::HandlerFunctionArray,
        JsPatternKind::VmFrameStackLocals,
        JsPatternKind::WebAssemblyBridge,
        JsPatternKind::BlobWorkerPath,
    ] {
        assert!(report.patterns.iter().any(|finding| finding.kind == kind));
    }
}

#[test]
fn classifies_lexically_isolated_handlers_and_infers_frame_members() {
    let repeated = (0..8)
        .map(|index| {
            format!(
                "function(frame) {{ var value = frame.values[frame.values.length - 1]; frame.slots.store({index}, value); frame.values[frame.values.length] = frame.slots.load({index}); }}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r"
        const fromCode = String.fromCharCode;
        function Stack() {{}}
        function Frame() {{ this.values = Stack(); this.failures = Stack(); }}
        const handlers = [
            {repeated},
            function(frame, key) {{ return frame.values[0][key]; }},
            function(frame, fn, receiver, args) {{ return fn.apply(receiver, args); }},
            function(frame, Ctor, args) {{ return Reflect.construct(Ctor, args); }},
            function(frame, code) {{ return fromCode(code); }},
            function(frame) {{ try {{ return frame.values[0]; }} catch (error) {{ frame.failures[0] = error; }} }}
        ];
        "
    );
    let report = analyze(&source);

    assert_eq!(report.handler_candidates.len(), 13);
    assert!(report.handler_candidates.iter().any(|candidate| {
        candidate.roles.contains(&HandlerRole::StackRead)
            && candidate.roles.contains(&HandlerRole::StackWrite)
            && candidate.roles.contains(&HandlerRole::LocalRead)
            && candidate.roles.contains(&HandlerRole::LocalWrite)
    }));
    for role in [
        HandlerRole::Call,
        HandlerRole::Construct,
        HandlerRole::PropertyAccess,
        HandlerRole::StringDecode,
    ] {
        assert!(
            report
                .handler_candidates
                .iter()
                .any(|candidate| candidate.roles.contains(&role))
        );
    }
    for role in [
        SymbolRole::HandlerArray,
        SymbolRole::OperandStack,
        SymbolRole::ExceptionStack,
        SymbolRole::LocalStore,
    ] {
        assert!(
            report
                .symbol_role_suggestions
                .iter()
                .any(|suggestion| suggestion.role == role)
        );
    }
    assert!(
        report
            .role_suggestions
            .iter()
            .all(|suggestion| suggestion.rationale.contains("not rewritten"))
    );
}

#[test]
fn extracts_literal_wasm_through_local_decoder_and_parses_it() {
    // One imported function `m.h`, one defined/exported function `f`, and a call to import 0.
    let wasm = [
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02, 0x07, 0x01, 0x01,
        b'm', 0x01, b'h', 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0x07, 0x05, 0x01, 0x01, b'f', 0x00, 0x01, 0x0a, 0x06,
        0x01, 0x04, 0x00, 0x10, 0x00, 0x0b,
    ];
    let encoded = STANDARD.encode(wasm);
    let source = format!(
        r#"
        function decodeBase64(value) {{ return atob(value); }}
        const bytes = decodeBase64("{encoded}");
        const module = new WebAssembly.Module(bytes);
        "#
    );
    let report = analyze(&source);

    assert_eq!(report.literal_folds.len(), 1);
    assert_eq!(report.literal_folds[0].kind, LiteralFoldKind::Base64Bytes);
    assert_eq!(report.wasm_modules.len(), 1);
    let module = &report.wasm_modules[0];
    assert!(module.errors.is_empty(), "{:?}", module.errors);
    assert_eq!(module.imports[0].module, "m");
    assert_eq!(module.imports[0].name, "h");
    assert_eq!(module.imports[0].function_index, Some(0));
    assert_eq!(module.exports[0].name, "f");
    assert!(
        module
            .operators
            .iter()
            .any(|operator| operator.kind == "Call" && operator.operand == Some(0))
    );
    assert!(
        module.structured_control[0]
            .edges
            .iter()
            .any(|edge| edge.kind == StructuredControlEdgeKind::CallReference && edge.target_index == Some(0))
    );
}

#[test]
fn structured_control_graph_records_blocks_and_branch_references() {
    let wasm = [
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00,
        0x0a, 0x09, 0x01, 0x07, 0x00, 0x02, 0x40, 0x0c, 0x00, 0x0b, 0x0b,
    ];
    let source = format!(
        "function b64(x){{return atob(x);}} const m=b64('{}');",
        STANDARD.encode(wasm)
    );
    let report = analyze(&source);
    let structure = &report.wasm_modules[0].structured_control[0];
    assert_eq!(structure.nodes.len(), 2);
    assert!(
        structure
            .edges
            .iter()
            .any(|edge| edge.kind == StructuredControlEdgeKind::Contains)
    );
    assert!(
        structure
            .edges
            .iter()
            .any(|edge| edge.kind == StructuredControlEdgeKind::Branch)
    );
}

#[test]
fn does_not_extract_global_atob_dynamic_or_non_wasm_literals() {
    let source = r#"
        const direct = atob("AGFzbQEAAAA=");
        function local(x) { return atob(x); }
        const dynamic = local(payload);
        const text = local("aGVsbG8=");
    "#;
    let report = analyze(source);

    assert_eq!(report.literal_folds.len(), 1);
    assert_eq!(report.literal_folds[0].utf8_preview.as_deref(), Some("hello"));
    assert_eq!(report.wasm_modules, []);
}

#[test]
fn rejects_invalid_base64_and_escaped_or_concatenated_arguments() {
    let source = r#"
        function local(x) { return atob(x); }
        local("not base64!");
        local("AGFz" + "bQEAAAA=");
        local(`AGFzbQEAAAA=`);
    "#;
    let report = analyze(source);
    assert_eq!(report.literal_folds, []);
    assert_eq!(report.wasm_modules, []);
}

#[test]
fn detects_target_sized_arrays_and_nested_handler_bodies() {
    let pool = (0..1484)
        .map(|index| format!("\"value_{index}\""))
        .collect::<Vec<_>>()
        .join(",");
    let descriptors = (0..645)
        .map(|index| format!("{{ op: {index}, reads: [0], writes: [] }}"))
        .collect::<Vec<_>>()
        .join(",");
    let handlers = (0..198)
        .map(|index| {
            format!(
                "function(frame) {{ if (frame.q.length) {{ var value = frame.q[frame.q.length - 1]; frame.l.put({index}, value); frame.q[frame.q.length] = frame.l.get({index}); }} }}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("const literals=[{pool}]; const descriptors=[{descriptors}]; const handlers=[{handlers}];");
    let report = analyze(&source);

    let count_for = |kind| {
        report
            .patterns
            .iter()
            .find(|finding| finding.kind == kind)
            .and_then(|finding| finding.item_count)
    };
    assert_eq!(count_for(JsPatternKind::LargeLiteralPool), Some(1484));
    assert_eq!(count_for(JsPatternKind::DescriptorObjectArray), Some(645));
    assert_eq!(count_for(JsPatternKind::HandlerFunctionArray), Some(198));
    assert_eq!(report.handler_candidates.len(), 198);
    assert!(
        report
            .symbol_role_suggestions
            .iter()
            .any(|suggestion| { suggestion.role == SymbolRole::DescriptorArray && suggestion.symbol == "descriptors" })
    );
}

#[test]
fn recognizes_manual_decoder_and_unpadded_wasm_for_narrow_extraction() {
    let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    let encoded = STANDARD.encode(wasm).trim_end_matches('=').to_string();
    let source = format!(
        r#"
        function manual64(input) {{
            var alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            var output = [];
            for (var i = 0; i < input.length; i += 4) {{
                var a = alphabet.indexOf(input[i]);
                var b = alphabet.indexOf(input[i + 1]);
                var c = alphabet.indexOf(input[i + 2]);
                var d = alphabet.indexOf(input[i + 3]);
                output[i] = a << 2 | b >> 4;
                output[i + 1] = (b & 15) << 4 | c >> 2;
                output[i + 2] = (c & 3) << 6 | d;
            }}
            return output;
        }}
        const moduleBytes = manual64("{encoded}");
        "#
    );

    let extracted = extract_wasm(&source);
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].bytes, wasm);
    let report = analyze(&source);
    assert_eq!(report.literal_folds.len(), 1);
    assert_eq!(report.wasm_modules.len(), 1);
    assert!(
        report
            .symbol_role_suggestions
            .iter()
            .any(|suggestion| { suggestion.role == SymbolRole::Base64Decoder && suggestion.symbol == "manual64" })
    );
}

#[test]
fn semantic_decoder_resolution_rejects_text_member_calls_and_shadowing() {
    let source = r#"
        function decode(value) { return atob(value); }
        const text = "decode(\"aWdub3JlZA==\")";
        // decode("aWdub3JlZA==")
        object.decode("aWdub3JlZA==");
        decode("aGk=");
        {
            function decode(value) { return value; }
            decode("c2hhZG93ZWQ=");
        }
    "#;
    let report = analyze(source);
    assert_eq!(report.literal_folds.len(), 1);
    assert_eq!(report.literal_folds[0].utf8_preview.as_deref(), Some("hi"));
}

#[test]
fn decoder_shape_requires_direct_dataflow_and_returned_manual_buffer() {
    let source = r#"
        function noisy(value) { sideEffect(); return atob(value); }
        function wrongManual(input) {
            const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            const output = [], other = [];
            output[0] = 1 << 2 | 2 >> 4;
            output[1] = (2 & 15) << 4 | 3 >> 2;
            output[2] = (3 & 3) << 6 | 4;
            return other;
        }
        noisy("aGk=");
        wrongManual("aGk=");
    "#;
    assert_eq!(analyze(source).literal_folds, []);
}

#[test]
fn malformed_wasm_is_reported_and_not_extracted() {
    let malformed = STANDARD.encode(b"\0asm\x02\0\0\0");
    let source = format!("function decode(value){{return atob(value);}} const bytes=decode(\"{malformed}\");");
    let report = analyze(&source);
    assert_eq!(report.wasm_modules.len(), 1);
    assert!(!report.wasm_modules[0].valid);
    assert_ne!(report.wasm_modules[0].validation_errors, Vec::<String>::new());
    assert_eq!(extract_wasm(&source), []);
}

#[test]
fn padding_edges_are_conservative() {
    let source = r#"
        function decode(value) { return atob(value); }
        decode("YQ");
        decode("aGk");
        decode("A");
        decode("YQ=");
    "#;
    let report = analyze(source);
    assert_eq!(report.literal_folds.len(), 2);
    assert_eq!(report.literal_folds[0].utf8_preview.as_deref(), Some("a"));
    assert_eq!(report.literal_folds[1].utf8_preview.as_deref(), Some("hi"));
}

#[test]
fn aggregate_budget_stops_analysis_with_one_truncation_diagnostic() {
    let source = r#"
        function decode(value) { return atob(value); }
        decode("AGFzbQEAAAA=");
    "#;
    let limits = AnalysisLimits {
        max_decoded_bytes: 4,
        ..AnalysisLimits::default()
    };
    let report = analyze_with_limits(source, limits);
    assert_eq!(report.literal_folds, []);
    assert_eq!(
        report.outcome,
        AnalysisOutcome::Truncated {
            reason: TruncatedAnalysisReason::AggregateBudgetExhausted,
        }
    );
}

#[test]
fn report_payload_budget_drops_heavy_collections() {
    let handlers = (0..64)
        .map(|_| "function(frame){frame.stack[frame.stack.length]=frame.stack[0];}")
        .collect::<Vec<_>>()
        .join(",");
    let source = format!("const handlers=[{handlers}];");
    let limits = AnalysisLimits {
        max_report_bytes: 1_024,
        ..AnalysisLimits::default()
    };
    let report = analyze_with_limits(&source, limits);
    assert!(serde_json::to_vec(&report).expect("serialize bounded report").len() <= 1_024);
    assert_eq!(
        report.outcome,
        AnalysisOutcome::Truncated {
            reason: TruncatedAnalysisReason::ReportPayloadLimitExceeded,
        }
    );
}

#[test]
fn report_is_versioned_and_serde_serializable() {
    let report = analyze("const handlers = [x => x, x => x, x => x, x => x];");
    let json = serde_json::to_value(&report).expect("report should serialize");
    assert_eq!(json["schema_version"], VM_ANALYSIS_SCHEMA_VERSION);
    assert_eq!(json["outcome"]["status"], "completed_findings");
    assert!(json.get("patterns").is_some());
    assert!(json.get("wasm_modules").is_some());
    assert!(json.get("role_suggestions").is_some());
}

#[test]
fn artifact_reuses_one_budget_for_report_and_exact_extraction() {
    let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    let encoded = STANDARD.encode(wasm);
    let source = format!("function b64(x){{return atob(x);}} const m=b64('{encoded}');");
    let limits = AnalysisLimits {
        max_modules: 1,
        max_decoded_bytes: wasm.len(),
        ..AnalysisLimits::default()
    };

    let artifact = analyze_artifact_with_limits(&source, limits);

    assert_eq!(artifact.report.outcome, AnalysisOutcome::CompletedFindings);
    assert_eq!(artifact.report.wasm_modules.len(), 1);
    assert_eq!(artifact.extracted_wasm.len(), 1);
    assert_eq!(artifact.extracted_wasm[0].bytes, wasm);
    assert_eq!(
        artifact.extracted_wasm[0].source_span,
        artifact.report.wasm_modules[0].source_span
    );
}

#[test]
fn artifact_keeps_extracted_bytes_out_of_serialized_report() {
    let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    let source = format!(
        "function b64(x){{return atob(x);}} const m=b64('{}');",
        STANDARD.encode(wasm)
    );
    let artifact = analyze_artifact_with_limits(&source, AnalysisLimits::default());
    let json = serde_json::to_value(&artifact.report).expect("serialize report");

    assert_eq!(artifact.extracted_wasm[0].bytes, wasm);
    assert!(json.get("extracted_wasm").is_none());
    assert!(!json.to_string().contains("[0,97,115,109"));
}

#[test]
fn source_policy_rejection_is_typed_not_analyzed() {
    let artifact = analyze_artifact_with_limits(
        "too large",
        AnalysisLimits {
            max_source_bytes: 3,
            ..AnalysisLimits::default()
        },
    );

    assert_eq!(
        artifact.report.outcome,
        AnalysisOutcome::NotAnalyzed {
            reason: NotAnalyzedReason::SourceTooLarge {
                actual_bytes: 9,
                limit_bytes: 3,
            },
        }
    );
    assert_eq!(artifact.extracted_wasm, []);
}

#[test]
fn empty_valid_input_has_completed_none_outcome() {
    assert_eq!(analyze("").outcome, AnalysisOutcome::CompletedNone);
}
