use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::budget::AnalysisBudget;
use super::report::{LiteralFold, LiteralFoldKind, SourceSpan, SymbolRole, SymbolRoleSuggestion};
use super::syntax::SyntaxFacts;

const MAX_LITERAL_BYTES: usize = 8 * 1024 * 1024;
const PREVIEW_CHARS: usize = 96;

pub(super) struct DecodedLiteral {
    pub(super) span: SourceSpan,
    pub(super) bytes: Vec<u8>,
}

#[derive(Default)]
pub(super) struct LiteralAnalysis {
    pub(super) folds: Vec<LiteralFold>,
    pub(super) decoded: Vec<DecodedLiteral>,
    pub(super) decoder_suggestions: Vec<SymbolRoleSuggestion>,
    pub(super) diagnostics: Vec<String>,
}

pub(super) fn fold_local_base64_calls(facts: &SyntaxFacts, budget: &mut AnalysisBudget) -> LiteralAnalysis {
    let decoders = facts
        .decoders
        .iter()
        .map(|decoder| (decoder.name.as_str(), decoder))
        .collect::<BTreeMap<_, _>>();
    let mut used_decoders = BTreeSet::new();
    let mut folds = Vec::new();
    let mut decoded = Vec::new();
    let mut rejected = 0_usize;

    for call in &facts.decoder_calls {
        if budget.exhausted() {
            break;
        }
        if call.literal.len() > MAX_LITERAL_BYTES.saturating_mul(4) / 3 + 4 {
            rejected += 1;
            continue;
        }
        let Some(bytes) = decode_unpadded(&call.literal) else {
            rejected += 1;
            continue;
        };
        if bytes.len() > MAX_LITERAL_BYTES || !budget.retain_decoded_bytes(bytes.len()) {
            break;
        }
        let is_wasm = bytes.starts_with(b"\0asm");
        let utf8_preview = (!is_wasm).then(|| bounded_utf8_preview(&bytes)).flatten();
        folds.push(LiteralFold {
            decoder: call.decoder.clone(),
            call_span: call.span,
            kind: if utf8_preview.is_some() && !is_wasm {
                LiteralFoldKind::Base64Utf8
            } else {
                LiteralFoldKind::Base64Bytes
            },
            byte_len: bytes.len(),
            digest_md5: format!("{:x}", md5::compute(&bytes)),
            utf8_preview,
        });
        decoded.push(DecodedLiteral { span: call.span, bytes });
        used_decoders.insert(call.decoder.as_str());
    }

    let decoder_suggestions = used_decoders
        .into_iter()
        .filter_map(|name| decoders.get(name))
        .map(|decoder| SymbolRoleSuggestion {
            symbol: decoder.name.clone(),
            span: Some(decoder.name_span),
            role: SymbolRole::Base64Decoder,
            suggested_name: "decode_base64_bytes".to_string(),
            rationale: decoder.rationale.clone(),
        })
        .collect::<Vec<_>>();
    let mut diagnostics = vec![format!(
        "recognized {} local base64 decoder(s); folded {} direct literal call(s)",
        decoder_suggestions.len(),
        folds.len()
    )];
    if rejected > 0 {
        diagnostics.push(format!(
            "ignored {rejected} invalid or over-limit literal base64 call(s)"
        ));
    }
    LiteralAnalysis {
        folds,
        decoded,
        decoder_suggestions,
        diagnostics,
    }
}

fn bounded_utf8_preview(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut preview = String::new();
    for character in text.chars().take(PREVIEW_CHARS) {
        preview.extend(character.escape_default());
    }
    if text.chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    Some(preview)
}

fn decode_unpadded(literal: &str) -> Option<Vec<u8>> {
    let remainder = literal.len() % 4;
    if remainder == 1 || (literal.contains('=') && remainder != 0) {
        return None;
    }
    if remainder == 0 {
        return STANDARD.decode(literal).ok();
    }
    let mut padded = String::with_capacity(literal.len() + (4 - remainder));
    padded.push_str(literal);
    padded.extend(std::iter::repeat_n('=', 4 - remainder));
    STANDARD.decode(padded).ok()
}
