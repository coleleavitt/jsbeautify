use rustc_hash::{FxHashMap, FxHashSet};

use crate::ast_deobfuscate::dowhile_switch_detector::{DoWhileDispatcher, StateValue};

/// Per case of `dispatcher`, whether some call can run it. Every case is
/// reachable when no call is known.
pub(super) fn reachable_cases(dispatcher: &DoWhileDispatcher) -> Vec<bool> {
    let cases = &dispatcher.cases;
    if dispatcher.entries.is_empty() {
        return vec![true; cases.len()];
    }
    // `switch` runs the first case whose label is `===` the state.
    let mut selects: FxHashMap<&StateValue, usize> = FxHashMap::default();
    for (index, case) in cases.iter().enumerate() {
        selects.entry(&case.label).or_insert(index);
    }
    let mut reached = vec![false; cases.len()];
    let mut seen: FxHashSet<&StateValue> = FxHashSet::default();
    let mut values: Vec<&StateValue> = dispatcher.entries.iter().collect();
    while let Some(value) = values.pop() {
        if !seen.insert(value) {
            continue;
        }
        let Some(&first) = selects.get(value) else {
            continue;
        };
        for (index, case) in cases.iter().enumerate().skip(first) {
            // A case reached before has queued its values and followed its
            // fall-through already.
            if reached[index] {
                break;
            }
            reached[index] = true;
            values.extend(&case.next);
            if !case.falls_through {
                break;
            }
        }
    }
    reached
}
