//! Webpack module ID → package name resolver.
//!
//! Maps numeric webpack module IDs to their source package by analyzing:
//! - Import patterns (`f.d(Ee, {Z: () => jn})` = re-export from module)
//! - Known Angular core module IDs (stable across builds due to deterministic hashing)
//! - String constants that reveal package identity (e.g. "ɵɵdefineComponent" = @angular/core)
//! - Library-specific patterns (RxJS Observable, NgRx Store, etc.)
//!
//! For the Schwab SWS Gateway (webpack 5.94, moduleIds: 'deterministic'):
//! Module IDs are stable across builds of the same code — same source = same ID.

use regex::Regex;
use rustc_hash::FxHashMap;

/// A resolved webpack module.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub id: u32,
    pub package: String,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Matched by unique string/pattern (very reliable)
    High,
    /// Matched by structural heuristic
    Medium,
    /// Guessed from context
    Low,
}

/// Result of module resolution.
#[derive(Debug)]
pub struct ModuleResolutionResult {
    pub resolved: FxHashMap<u32, ResolvedModule>,
    pub unresolved_count: usize,
    pub total_modules: usize,
}

/// Resolve webpack module IDs to package names from the bundled source.
#[must_use]
pub fn resolve_module_ids(source: &str) -> ModuleResolutionResult {
    lazy_static::lazy_static! {
        // Module definition: NNNN: (exports, module, require) => { ... }
        static ref MODULE_DEF_RE: Regex = Regex::new(
            r"(\d{1,5}):\s*\([^)]*\)\s*=>\s*\{"
        ).unwrap();

        // Webpack require call: f(NNNN) or t(NNNN) or __webpack_require__(NNNN)
        static ref REQUIRE_CALL_RE: Regex = Regex::new(
            r"\b\w\((\d{1,5})\)"
        ).unwrap();
    }

    let mut resolved: FxHashMap<u32, ResolvedModule> = FxHashMap::default();
    let mut total_modules = 0usize;

    // Split source into module regions and analyze each
    let module_regions = extract_module_regions(source);
    total_modules = module_regions.len();

    for (id, region) in &module_regions {
        if let Some(package) = identify_module_package(region) {
            resolved.insert(
                *id,
                ResolvedModule {
                    id: *id,
                    package: package.0,
                    confidence: package.1,
                },
            );
        }
    }

    let unresolved_count = total_modules.saturating_sub(resolved.len());

    ModuleResolutionResult {
        resolved,
        unresolved_count,
        total_modules,
    }
}

/// Extract module ID → source region map from webpack bundle.
fn extract_module_regions(source: &str) -> Vec<(u32, String)> {
    lazy_static::lazy_static! {
        // Match module definitions at the start of object properties
        static ref MODULE_START_RE: Regex = Regex::new(
            r"(?m)^\s*(\d{1,5}):\s*\([^)]*\)\s*=>\s*\{"
        ).unwrap();
    }

    let mut regions = Vec::new();

    let starts: Vec<(u32, usize)> = MODULE_START_RE
        .captures_iter(source)
        .filter_map(|caps| {
            let id: u32 = caps.get(1)?.as_str().parse().ok()?;
            let start = caps.get(0)?.start();
            Some((id, start))
        })
        .collect();

    for i in 0..starts.len() {
        let (id, start) = starts[i];
        let end = if i + 1 < starts.len() {
            starts[i + 1].1
        } else {
            (start + 2000).min(source.len())
        };
        // Take first 1000 chars of each module for analysis (enough to identify)
        let region_end = (start + 1000).min(end).min(source.len());
        regions.push((id, source[start..region_end].to_string()));
    }

    regions
}

/// Identify what package a module region belongs to based on content.
fn identify_module_package(region: &str) -> Option<(String, Confidence)> {
    // Angular Core (has ɵɵ instruction definitions or references)
    if region.contains("ɵɵdefineComponent")
        || region.contains("ɵɵdefineDirective")
        || region.contains("ɵɵdefineInjectable")
        || region.contains("ɵɵdefinePipe")
        || region.contains("ɵɵdefineNgModule")
    {
        return Some(("@angular/core".into(), Confidence::High));
    }

    // Angular Router
    if region.contains("RouterModule")
        || region.contains("ActivatedRoute")
        || region.contains("routerLink")
        || region.contains("NavigationEnd")
    {
        return Some(("@angular/router".into(), Confidence::High));
    }

    // Angular Forms
    if region.contains("FormBuilder")
        || region.contains("FormControl")
        || region.contains("Validators")
        || region.contains("ngModel")
    {
        return Some(("@angular/forms".into(), Confidence::High));
    }

    // Angular Common/HTTP
    if region.contains("HttpClient") || region.contains("HttpHeaders") || region.contains("HttpInterceptor") {
        return Some(("@angular/common/http".into(), Confidence::High));
    }

    // Angular CDK
    if region.contains("CdkOverlay") || region.contains("cdkConnectedOverlay") || region.contains("A11yModule") {
        return Some(("@angular/cdk".into(), Confidence::Medium));
    }

    // RxJS
    if region.contains("Observable")
        || region.contains("subscribe")
        || region.contains("BehaviorSubject")
        || region.contains("ReplaySubject")
    {
        return Some(("rxjs".into(), Confidence::Medium));
    }

    // NgRx Store
    if region.contains("createAction")
        || region.contains("createReducer")
        || region.contains("createEffect")
        || region.contains("createSelector")
        || region.contains("@ngrx")
    {
        return Some(("@ngrx/store".into(), Confidence::High));
    }

    // NgRx Effects
    if region.contains("createEffect") || region.contains("ofType") {
        return Some(("@ngrx/effects".into(), Confidence::Medium));
    }

    // Transmit Security SDK
    if region.contains("XmSdk") || region.contains("xmsdk") || region.contains("riskid") {
        return Some(("@anthropic/transmit-security".into(), Confidence::High));
    }

    // LaunchDarkly
    if region.contains("LDClient") || region.contains("ldclient") || region.contains("variation(") {
        return Some(("launchdarkly-js-client-sdk".into(), Confidence::High));
    }

    // Zone.js
    if region.contains("Zone.__symbol__") || region.contains("__zone_symbol__") {
        return Some(("zone.js".into(), Confidence::High));
    }

    // tslib helpers
    if region.contains("__decorate")
        || region.contains("__metadata")
        || region.contains("__awaiter")
        || region.contains("__generator")
    {
        return Some(("tslib".into(), Confidence::Medium));
    }

    None
}

/// Annotate webpack require calls with package names.
#[must_use]
pub fn annotate_require_calls(source: &str, resolved: &FxHashMap<u32, ResolvedModule>) -> (String, usize) {
    if resolved.is_empty() {
        return (source.to_string(), 0);
    }

    let mut result = source.to_string();
    let mut count = 0usize;

    // Replace `f(NNNN)` with `f(NNNN /* @package/name */)`
    for (id, module) in resolved {
        let patterns = [
            format!("f({id})"),
            format!("t({id})"),
            format!("__webpack_require__({id})"),
        ];

        for pattern in &patterns {
            let replacement = format!("{pattern} /* {} */", module.package);
            if result.contains(pattern.as_str()) && !result.contains(&replacement) {
                result = result.replace(pattern.as_str(), &replacement);
                count += 1;
            }
        }
    }

    (result, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_angular_core() {
        let region = "var g = { ɵɵdefineComponent: function(){} }";
        assert_eq!(identify_module_package(region).unwrap().0, "@angular/core");
    }

    #[test]
    fn test_identify_rxjs() {
        let region = "class Observable { subscribe(observer) {} }";
        assert_eq!(identify_module_package(region).unwrap().0, "rxjs");
    }

    #[test]
    fn test_identify_ngrx() {
        let region = "function createAction(type, config) {}";
        assert_eq!(identify_module_package(region).unwrap().0, "@ngrx/store");
    }
}
