//! LLM-assisted naming tier (the [humanify] pattern).
//!
//! For minified bindings that the heuristic and sourcemap tiers couldn't name,
//! ask a language model for a descriptive name. The model only *suggests* a
//! string — the rename itself goes through the same scope-safe `SymbolId` path
//! ([`super::bun::local_renamer::RenameMode::Sourcemap`]), so a bad suggestion
//! can never break the program: every name is validated, collisions are
//! resolved, and unusable suggestions are dropped.
//!
//! Off by default. Enabled with `--rename-llm` plus an OpenAI-compatible
//! endpoint. Configuration is resolved by [`crate::llm`] — standard
//! `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `OPENAI_MODEL`, the project-specific
//! `JSBEAUTIFY_LLM_*` vars, or explicit `--llm-url/--llm-model/--llm-key` CLI
//! flags (which take precedence).
//!
//! [humanify]: https://github.com/jehna/humanify

use oxc_allocator::Allocator;
use oxc_ast::ast::BindingIdentifier;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{ReusableTraverseCtx, Traverse, TraverseCtx, traverse_mut_with_ctx};
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::bun::local_renamer::{
    LocalRenameMap, LocalRenamer, RenameMode, is_likely_generated_alias, is_meaningful_short, is_usable_base,
    plan_local_renames,
};
use crate::ast_deobfuscate::heuristic_rename::HeuristicNameCollector;
use crate::ast_deobfuscate::rename_guards::RenameGuardCollector;
use crate::ast_deobfuscate::state::DeobfuscateState;
use crate::llm::{LlmClient, LlmOverrides};

type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

/// Max characters of code context handed to the model per binding.
const CONTEXT_WINDOW: usize = 600;

/// A request to name one binding: its current (minified) name + a window of the
/// surrounding source.
#[derive(Debug, Clone)]
pub struct NameRequest {
    pub current_name: String,
    pub context: String,
}

/// Suggests readable names for minified bindings. The returned vector is
/// parallel to `requests`; `None` means "no suggestion" (leave the binding).
pub trait NameSuggester {
    fn suggest_batch(&self, requests: &[NameRequest]) -> Vec<Option<String>>;
}

// ── OpenAI-compatible HTTP provider ──────────────────────────────────────────

/// Suggester backed by the shared [`LlmClient`] (any OpenAI-compatible
/// endpoint: `OpenAI`, `OpenRouter`, Ollama, llama.cpp, …).
pub struct OpenAiCompatSuggester {
    client: LlmClient,
}

impl OpenAiCompatSuggester {
    /// Build from CLI overrides falling back to env (`OPENAI_*` /
    /// `JSBEAUTIFY_LLM_*`). Returns `None` when no endpoint is configured.
    #[must_use]
    pub fn from_overrides(ov: &LlmOverrides) -> Option<Self> {
        Some(Self {
            client: LlmClient::from_overrides(ov)?,
        })
    }

    /// Convenience: build purely from environment (no CLI overrides).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_overrides(&LlmOverrides::default())
    }
}

impl NameSuggester for OpenAiCompatSuggester {
    fn suggest_batch(&self, requests: &[NameRequest]) -> Vec<Option<String>> {
        if requests.is_empty() {
            return Vec::new();
        }
        let user_items: Vec<serde_json::Value> = requests
            .iter()
            .enumerate()
            .map(|(i, r)| serde_json::json!({ "i": i, "name": r.current_name, "context": r.context }))
            .collect();
        let system = "You rename minified JavaScript identifiers. Given a JSON array of \
            {i, name, context}, infer one concise, descriptive lowerCamelCase name per item \
            based on how the variable is used in its context. Keep already-meaningful names. \
            Use only valid JS identifiers; never reserved words. Respond with ONLY a JSON array \
            of strings, one per input, in the same order.";
        let user = serde_json::Value::Array(user_items).to_string();
        self.client.chat(system, &user).map_or_else(
            |_| vec![None; requests.len()],
            |content| parse_name_array(&content, requests.len()),
        )
    }
}

/// Parse the model's reply into exactly `n` optional names. Tolerates a JSON
/// array wrapped in prose / code fences.
fn parse_name_array(content: &str, n: usize) -> Vec<Option<String>> {
    let slice = match (content.find('['), content.rfind(']')) {
        (Some(a), Some(b)) if b > a => &content[a..=b],
        _ => return vec![None; n],
    };
    let Ok(names) = serde_json::from_str::<Vec<String>>(slice) else {
        return vec![None; n];
    };
    let mut out: Vec<Option<String>> = names
        .into_iter()
        .map(|s| {
            let s = s.trim().to_string();
            (is_usable_base(&s)).then_some(s)
        })
        .collect();
    out.resize(n, None);
    out
}

// ── Candidate collection ─────────────────────────────────────────────────────

/// Residual minified binding: still looks generated and isn't an idiomatic short.
struct LlmCandidateCollector {
    candidates: Vec<(SymbolId, String, u32)>, // (symbol, current name, decl byte offset)
}

impl LlmCandidateCollector {
    const fn new() -> Self {
        Self { candidates: Vec::new() }
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for LlmCandidateCollector {
    fn enter_binding_identifier(&mut self, it: &mut BindingIdentifier<'a>, _ctx: &mut Ctx<'a>) {
        let name = it.name.as_str();
        if is_likely_generated_alias(name)
            && !is_meaningful_short(name)
            && let Some(sym) = it.symbol_id.get()
        {
            self.candidates.push((sym, name.to_string(), it.span.start));
        }
    }
}

/// A bounded source window around `offset` (clamped to char boundaries).
fn context_window(source: &str, offset: u32) -> String {
    let offset = (offset as usize).min(source.len());
    let mut lo = offset.saturating_sub(CONTEXT_WINDOW / 5);
    let mut hi = (offset + CONTEXT_WINDOW * 4 / 5).min(source.len());
    while lo > 0 && !source.is_char_boundary(lo) {
        lo -= 1;
    }
    while hi < source.len() && !source.is_char_boundary(hi) {
        hi += 1;
    }
    source[lo..hi].to_string()
}

/// Rename residual minified bindings using `suggester`, scope-safely. Bindings
/// the suggester declines or that the soundness guards exclude are left alone.
#[must_use]
pub fn apply_llm_names(code: &str, source_type: SourceType, suggester: &(dyn NameSuggester + Sync)) -> String {
    crate::stack::run_on_sized_stack(code.len(), || {
        apply_llm_names_on_current_stack(code, source_type, suggester)
    })
}

fn apply_llm_names_on_current_stack(code: &str, source_type: SourceType, suggester: &dyn NameSuggester) -> String {
    let alloc = Allocator::default();
    let mut prog = Parser::new(&alloc, code, source_type).parse().program;

    // Guards (eval / native exports / script globals) — never rename these.
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut guard = RenameGuardCollector::new();
    traverse_mut_with_ctx(&mut guard, &mut prog, &mut ctx);
    drop(ctx);
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let guarded = guard.into_guarded(&scoping);

    // Capture-safety free names (heuristic collector; proposals discarded).
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut hc = HeuristicNameCollector::new();
    traverse_mut_with_ctx(&mut hc, &mut prog, &mut ctx);
    drop(ctx);
    let (_proposals, free_names) = hc.into_parts();

    // Collect residual candidates.
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut collector = LlmCandidateCollector::new();
    traverse_mut_with_ctx(&mut collector, &mut prog, &mut ctx);
    drop(ctx);

    let candidates: Vec<(SymbolId, String, u32)> = collector
        .candidates
        .into_iter()
        .filter(|(sym, _, _)| !guarded.contains(sym))
        .collect();
    if candidates.is_empty() {
        return code.to_string();
    }

    let requests: Vec<NameRequest> = candidates
        .iter()
        .map(|(_, name, off)| NameRequest {
            current_name: name.clone(),
            context: context_window(code, *off),
        })
        .collect();
    let suggestions = suggester.suggest_batch(&requests);

    let mut desired: LocalRenameMap = FxHashMap::default();
    for ((sym, cur, _), suggestion) in candidates.iter().zip(suggestions) {
        if let Some(name) = suggestion
            && name != *cur
            && is_usable_base(&name)
        {
            desired.insert(*sym, name);
        }
    }
    if desired.is_empty() {
        return code.to_string();
    }

    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let plan = plan_local_renames(&scoping, &guarded, RenameMode::Sourcemap, &desired, &free_names);
    if plan.is_empty() {
        return code.to_string();
    }
    let scoping = SemanticBuilder::new().build(&prog).semantic.into_scoping();
    let mut ctx = ReusableTraverseCtx::new(DeobfuscateState::new(), scoping, &alloc);
    let mut renamer = LocalRenamer::new(plan);
    traverse_mut_with_ctx(&mut renamer, &mut prog, &mut ctx);
    Codegen::new().build(&prog).code
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic suggester: maps current name → canned readable name.
    struct MockSuggester(FxHashMap<String, String>);
    impl NameSuggester for MockSuggester {
        fn suggest_batch(&self, requests: &[NameRequest]) -> Vec<Option<String>> {
            requests.iter().map(|r| self.0.get(&r.current_name).cloned()).collect()
        }
    }

    #[test]
    fn applies_model_suggestions_scope_safely() {
        let mut map = FxHashMap::default();
        map.insert("v0".to_string(), "handler".to_string());
        map.insert("v1".to_string(), "payload".to_string());
        let suggester = MockSuggester(map);

        let out = apply_llm_names("function v0(v1){ return process(v1) }", SourceType::mjs(), &suggester);
        assert!(out.contains("function handler(payload)"), "decls: {out}");
        assert!(out.contains("return process(payload)"), "refs follow: {out}");
    }

    #[test]
    fn skips_eval_visible_bindings_even_with_a_suggestion() {
        let mut map = FxHashMap::default();
        map.insert("a".to_string(), "secret".to_string());
        let suggester = MockSuggester(map);
        // `a` is visible to a direct eval → must NOT be renamed.
        let out = apply_llm_names("function f(a){ return eval('a') }", SourceType::mjs(), &suggester);
        assert!(out.contains("(a)"), "eval-visible binding renamed: {out}");
    }

    #[test]
    fn parse_name_array_tolerates_fences_and_validates() {
        let names = parse_name_array("Here you go:\n```json\n[\"foo\", \"if\", \"barBaz\"]\n```", 3);
        assert_eq!(names[0].as_deref(), Some("foo"));
        assert_eq!(names[1], None); // `if` is reserved → rejected
        assert_eq!(names[2].as_deref(), Some("barBaz"));
    }

    #[test]
    fn invalid_response_yields_no_renames() {
        assert_eq!(parse_name_array("not json", 2), vec![None, None]);
    }
}
