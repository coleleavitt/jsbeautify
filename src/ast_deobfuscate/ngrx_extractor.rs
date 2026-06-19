//! `NgRx` state management extractor — identifies actions, reducers, effects, and selectors.
//!
//! Angular apps using @ngrx/store compile to patterns like:
//! ```js
//! // Action creation (createAction):
//! const loginSuccess = createAction("[Auth] Login Success", props<{token: string}>());
//! // Compiles to:
//! const Xs = (0, n.createAction)("[Auth] Login Success", (0, n.props)());
//!
//! // Reducer (createReducer):
//! const authReducer = createReducer(initialState, on(loginSuccess, (s, {token}) => ({...s, token})))
//! // Effects (createEffect):
//! login$ = createEffect(() => this.actions$.pipe(ofType(login), switchMap(...)))
//! ```
//!
//! This module extracts action type strings and maps them to their minified variable names.

use regex::Regex;
use std::fmt::Write as _;
use std::sync::LazyLock;

// createAction("[Type String]") or (0, n.createAction)("[Type String]", ...)
static ACTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:createAction|\w+\.createAction)\)\(\s*"(\[[^\]]*\]\s*[^"]*)"|\bcreateAction\(\s*"(\[[^\]]*\]\s*[^"]*)""#,
    )
    .unwrap()
});

// Variable assignment: const/let/var X = ...createAction(...)
static ACTION_VAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:const|let|var)\s+(\w+)\s*=\s*(?:\([^)]*\)\s*)?(?:\w+\.)?createAction\(\s*"(\[[^\]]*\][^"]*)"(?:\s*,\s*(?:\([^)]*\)\.)?props\(\))?"#,
    )
    .unwrap()
});

// store.dispatch(new ActionName(...)) or store.dispatch(actionCreator(...))
static DISPATCH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.dispatch\(\s*(?:new\s+)?(\w[\w.]*)\(").unwrap());

// createEffect(() => this.actions$.pipe(ofType(X, Y), ...))
static EFFECT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"createEffect\(\s*\(\)\s*=>\s*(?:this\.)?(?:actions?\$?)\.pipe\(\s*ofType\(([^)]+)\)").unwrap()
});

// ofType("action string") or ofType(actionVar)
static OFTYPE_STRING_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""(\[[^\]]*\][^"]*)""#).unwrap());

/// An extracted `NgRx` action.
#[derive(Debug, Clone)]
pub struct NgrxAction {
    /// The action type string, e.g. "[Auth] Login Success"
    pub action_type: String,
    /// The minified variable name, e.g. "Xs"
    pub minified_name: Option<String>,
    /// Whether it has props (payload)
    pub has_props: bool,
}

/// An extracted `NgRx` effect pattern.
#[derive(Debug, Clone)]
pub struct NgrxEffect {
    /// The action type(s) this effect responds to
    pub trigger_actions: Vec<String>,
    /// Whether it dispatches another action
    pub dispatches: bool,
}

/// Result of `NgRx` extraction.
#[derive(Debug)]
pub struct NgrxExtractionResult {
    pub actions: Vec<NgrxAction>,
    pub effects: Vec<NgrxEffect>,
    pub store_dispatches: Vec<String>,
}

/// Extract `NgRx` patterns from beautified source.
#[must_use]
pub fn extract_ngrx_patterns(source: &str) -> NgrxExtractionResult {
    let mut actions = Vec::new();
    let mut effects = Vec::new();
    let mut store_dispatches = Vec::new();

    // Extract actions with type strings
    for caps in ACTION_RE.captures_iter(source) {
        let action_type = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map_or("", |m| m.as_str())
            .to_string();
        let has_props = caps.get(0).map_or("", |m| m.as_str()).contains("props");

        if !action_type.is_empty() {
            actions.push(NgrxAction {
                action_type,
                minified_name: None,
                has_props,
            });
        }
    }

    // Extract actions with their variable names
    for caps in ACTION_VAR_RE.captures_iter(source) {
        let var_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        let action_type = caps.get(2).map_or("", |m| m.as_str()).to_string();
        let has_props = caps.get(0).map_or("", |m| m.as_str()).contains("props");

        // Update existing or add new
        let existing = actions.iter_mut().find(|a| a.action_type == action_type);
        if let Some(entry) = existing {
            entry.minified_name = Some(var_name);
        } else {
            actions.push(NgrxAction {
                action_type,
                minified_name: Some(var_name),
                has_props,
            });
        }
    }

    // Extract store dispatches
    for caps in DISPATCH_RE.captures_iter(source) {
        let action_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
        store_dispatches.push(action_name);
    }

    // Extract effects
    for caps in EFFECT_RE.captures_iter(source) {
        let oftype_args = caps.get(1).map_or("", |m| m.as_str());
        let mut trigger_actions = Vec::new();

        // Parse the ofType arguments — could be string literals or variable refs
        for string_cap in OFTYPE_STRING_RE.captures_iter(oftype_args) {
            trigger_actions.push(string_cap.get(1).map_or("", |m| m.as_str()).to_string());
        }

        if trigger_actions.is_empty() {
            // Just capture the raw argument text
            trigger_actions.push(oftype_args.trim().to_string());
        }

        effects.push(NgrxEffect {
            trigger_actions,
            dispatches: true, // Conservative assumption
        });
    }

    // Deduplicate actions
    actions.sort_by_key(|action| action.action_type.clone());
    actions.dedup_by(|a, b| a.action_type == b.action_type);

    NgrxExtractionResult {
        actions,
        effects,
        store_dispatches,
    }
}

/// Format extracted `NgRx` state as an annotation block.
#[must_use]
pub fn format_ngrx_summary(result: &NgrxExtractionResult) -> String {
    if result.actions.is_empty() && result.store_dispatches.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("// ╔══════════════════════════════════════════════════════════╗\n");
    output.push_str("// ║              NgRx STATE MANAGEMENT                      ║\n");
    output.push_str("// ╠══════════════════════════════════════════════════════════╣\n");

    if !result.actions.is_empty() {
        output.push_str("// ║ ACTIONS:                                                 ║\n");
        for action in &result.actions {
            let props_marker = if action.has_props { " (+ payload)" } else { "" };
            let var_marker = action
                .minified_name
                .as_deref()
                .map_or(String::new(), |n| format!(" [{n}]"));
            let _ = writeln!(output, "// ║   {}{}{}", action.action_type, props_marker, var_marker);
        }
    }

    if !result.effects.is_empty() {
        output.push_str("// ║ EFFECTS:\n");
        for effect in &result.effects {
            let _ = writeln!(output, "// ║   triggers on: {}", effect.trigger_actions.join(", "));
        }
    }

    let _ = writeln!(
        output,
        "// ║ Total: {} actions, {} effects, {} dispatches\n",
        result.actions.len(),
        result.effects.len(),
        result.store_dispatches.len()
    );
    output.push_str("// ╚══════════════════════════════════════════════════════════╝\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_actions() {
        let source = r#"
            const Xs = (0, n.createAction)("[Auth] Login Success", (0, n.props)());
            const Ys = (0, n.createAction)("[Auth] Login Failure");
            const Zs = (0, n.createAction)("[Auth] Logout");
        "#;

        let result = extract_ngrx_patterns(source);
        assert!(result.actions.len() >= 3);
        assert!(result.actions.iter().any(|a| a.action_type == "[Auth] Login Success"));
        assert!(result.actions.iter().any(|a| a.action_type == "[Auth] Logout"));
    }

    #[test]
    fn test_extract_dispatches() {
        let source = r"
            this.store.dispatch(new K.FjN({ ostEventCode: Z.fc.MAGuidancePopupOpen }));
            this.store.dispatch(loginSuccess({ token: t }));
        ";

        let result = extract_ngrx_patterns(source);
        assert_eq!(result.store_dispatches.len(), 2);
        assert!(result.store_dispatches.iter().any(|d| d.contains("FjN")));
    }
}
