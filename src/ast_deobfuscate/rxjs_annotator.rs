//! `RxJS` operator chain deobfuscator — identifies `pipe()` chains and annotates operators.
//!
//! Angular services heavily use `RxJS` patterns:
//! ```js
//! // Minified:
//! this.http.get(url).pipe((0,n.map)(r => r.data), (0,n.catchError)(e => of(null)))
//!
//! // After deobfuscation:
//! this.http.get(url).pipe(
//!   /* rxjs:map */ (0,n.map)(r => r.data),
//!   /* rxjs:catchError */ (0,n.catchError)(e => of(null))
//! )
//! ```
//!
//! Known `RxJS` operators (from rxjs 7.8.1):
//! Creation: of, from, fromEvent, interval, timer, merge, combineLatest, forkJoin
//! Transform: map, switchMap, mergeMap, concatMap, exhaustMap, scan, reduce
//! Filter: filter, take, takeUntil, takeWhile, skip, distinctUntilChanged, debounceTime
//! Utility: tap, delay, timeout, finalize, share, shareReplay
//! Error: catchError, retry, retryWhen
//! Combine: withLatestFrom, combineLatestWith, startWith, pairwise

use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::LazyLock;

/// Known `RxJS` operator names for detection.
const RXJS_OPERATORS: &[&str] = &[
    // Creation
    "of",
    "from",
    "fromEvent",
    "interval",
    "timer",
    "merge",
    "combineLatest",
    "forkJoin",
    "concat",
    "race",
    "zip",
    "EMPTY",
    "NEVER",
    "throwError",
    // Pipeable - Transform
    "map",
    "switchMap",
    "mergeMap",
    "concatMap",
    "exhaustMap",
    "scan",
    "reduce",
    "pluck",
    "mapTo",
    "switchMapTo",
    "mergeMapTo",
    // Pipeable - Filter
    "filter",
    "take",
    "takeUntil",
    "takeWhile",
    "skip",
    "skipUntil",
    "skipWhile",
    "first",
    "last",
    "distinctUntilChanged",
    "distinctUntilKeyChanged",
    "debounceTime",
    "debounce",
    "throttleTime",
    "throttle",
    "auditTime",
    "audit",
    "sampleTime",
    "sample",
    // Pipeable - Utility
    "tap",
    "delay",
    "delayWhen",
    "timeout",
    "timeoutWith",
    "finalize",
    "observeOn",
    "subscribeOn",
    "repeat",
    "repeatWhen",
    // Pipeable - Share/Multicast
    "share",
    "shareReplay",
    "publish",
    "publishReplay",
    "publishBehavior",
    "multicast",
    "refCount",
    "connect",
    // Pipeable - Error handling
    "catchError",
    "retry",
    "retryWhen",
    // Pipeable - Combine
    "withLatestFrom",
    "combineLatestWith",
    "startWith",
    "endWith",
    "pairwise",
    "bufferCount",
    "bufferTime",
    "buffer",
    "window",
    "windowCount",
    // Pipeable - Conditional
    "defaultIfEmpty",
    "every",
    "find",
    "findIndex",
    "isEmpty",
    // Pipeable - Math
    "count",
    "max",
    "min",
    // NgRx-specific
    "ofType",
    "select",
    "dispatch",
];

/// Detect `RxJS` operators in `pipe()` chains and annotate them.
/// Returns (`annotated_source`, `operator_count`).
#[must_use]
pub fn annotate_rxjs_operators(source: &str) -> (String, usize) {
    // Matches: (0, n.operatorName)( or n.operatorName( where operatorName is known
    static OPERATOR_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\(0,\s*\w+\.(\w+)\)\(|\b\w+\.(map|filter|switchMap|mergeMap|concatMap|exhaustMap|tap|catchError|take|takeUntil|first|last|distinctUntilChanged|debounceTime|throttleTime|share|shareReplay|withLatestFrom|startWith|finalize|retry|scan|reduce|combineLatestWith|ofType|select|skip|delay|timeout|pairwise|bufferCount|auditTime|sampleTime)\("
        ).unwrap()
    });

    let known_ops: FxHashSet<&str> = RXJS_OPERATORS.iter().copied().collect();
    let mut result = String::with_capacity(source.len() + source.len() / 20);
    let mut op_count = 0usize;

    for line in source.lines() {
        // NOTE: previously this bailed out on any line containing `.pipe(`,
        // which skipped annotation on exactly the lines that carry the operator
        // chain (single-line `x.pipe(map(a), filter(b))` is the common minified
        // shape). Operator detection now runs on every line.

        // Look for operator calls in the line
        let mut annotated_line = line.to_string();
        let mut found_in_line = false;

        for caps in OPERATOR_CALL_RE.captures_iter(line) {
            let op_name = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str());
            if let Some(name) = op_name
                && known_ops.contains(name)
                && !found_in_line
            {
                // Add annotation comment before this operator usage
                let indent = line.len() - line.trim_start().len();
                let pad = " ".repeat(indent);
                annotated_line = format!("{pad}/* rxjs:{name} */ {}", line.trim_start());
                found_in_line = true;
                op_count += 1;
            }
        }

        result.push_str(&annotated_line);
        result.push('\n');
    }

    (result, op_count)
}

/// Extract a summary of all `RxJS` operators used in the source.
#[must_use]
pub fn extract_operator_usage(source: &str) -> Vec<(String, usize)> {
    // Matches an operator call in both the bare member form `n.map(` and the
    // minified interop form `(0,n.map)(` (note the `)` between the operator
    // name and the call paren). Group 1 = operator name.
    static ALL_OPS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:\(0,\s*)?\w+\.(\w+)\s*\)?\s*\(").unwrap());

    let known_ops: FxHashSet<&str> = RXJS_OPERATORS.iter().copied().collect();

    // Count actual operator *call* sites (`n.map(`, `(0,n.map)(`), not bare
    // substring occurrences of the name: operator names like `of`/`map`/`share`
    // are common substrings (`offset`, `mapping`, `shareReplay`) and a raw
    // `source.matches(name)` grossly over-counts them.
    let mut counts: FxHashMap<String, usize> = FxHashMap::default();
    for caps in ALL_OPS_RE.captures_iter(source) {
        let name = caps.get(1).map_or("", |m| m.as_str());
        if known_ops.contains(name) {
            *counts.entry(name.to_string()).or_insert(0) += 1;
        }
    }

    let mut usage: Vec<(String, usize)> = counts.into_iter().collect();
    usage.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotate_pipe_operators() {
        let source = r"
            this.http.get(url).pipe(
                (0, n.map)(r => r.data),
                (0, n.catchError)(e => (0, rxjs.of)(null))
            );
        ";

        let (annotated, count) = annotate_rxjs_operators(source);
        assert!(count >= 2);
        assert!(annotated.contains("/* rxjs:map */"));
        assert!(annotated.contains("/* rxjs:catchError */"));
    }

    #[test]
    fn test_operator_usage_extraction() {
        let source = r"
            x.pipe(n.map(a), n.filter(b), n.switchMap(c), n.map(d))
        ";

        let usage = extract_operator_usage(source);
        assert_ne!(usage, []);
        // map appears twice
        let map_usage = usage.iter().find(|(name, _)| name == "map");
        assert!(map_usage.is_some());
    }

    #[test]
    fn test_operator_usage_counts_interop_form() {
        // Minified interop call sites `(0,n.op)(...)` must be counted, not just
        // the bare `n.op(` member form.
        let source = r"x.pipe((0,n.map)(a), (0,n.map)(b), (0,n.filter)(c))";
        let usage = extract_operator_usage(source);
        let map = usage.iter().find(|(name, _)| name == "map").expect("map counted");
        assert_eq!(map.1, 2, "interop map calls miscounted: {usage:?}");
        assert!(
            usage.iter().any(|(name, _)| name == "filter"),
            "interop filter missed: {usage:?}"
        );
    }
}
