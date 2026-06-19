use rustc_hash::{FxHashMap, FxHashSet};

use super::{DoWhileDispatcherMap, StateTransition};

pub(super) fn compute_reachability(
    dispatchers: &DoWhileDispatcherMap,
    entry_states: &FxHashMap<String, FxHashSet<String>>,
) -> FxHashMap<String, FxHashSet<String>> {
    let mut result: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();

    for (name, info) in dispatchers {
        let mut reachable = FxHashSet::default();

        // Seed with entry states
        if let Some(entries) = entry_states.get(name) {
            for entry in entries {
                reachable.insert(entry.clone());
            }
        }
        // Also check alt_name entries
        if let Some(alt) = &info.alt_name
            && let Some(entries) = entry_states.get(alt.as_str())
        {
            for entry in entries {
                reachable.insert(entry.clone());
            }
        }

        if reachable.is_empty() {
            // No known entry points — conservatively keep all cases
            for case in &info.cases {
                reachable.insert(case.label.clone());
            }
            result.insert(name.clone(), reachable);
            continue;
        }

        // Build transition map: label -> set of successor labels
        let mut transitions: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
        for case in &info.cases {
            let succs = match &case.transition {
                StateTransition::Sequential(next) => vec![next.as_str()],
                StateTransition::Conditional(a, b) => vec![a.as_str(), b.as_str()],
                StateTransition::LoopExit | StateTransition::Return | StateTransition::Compound { .. } => vec![],
                StateTransition::Unknown => {
                    // Conservative: all other case labels are potential targets
                    info.cases
                        .iter()
                        .filter(|c| c.label != case.label)
                        .map(|c| c.label.as_str())
                        .collect()
                }
            };
            transitions.insert(case.label.as_str(), succs);
        }

        // Fixpoint: expand reachable set by following transitions
        let mut worklist: Vec<String> = reachable.iter().cloned().collect();
        while let Some(label) = worklist.pop() {
            if let Some(succs) = transitions.get(label.as_str()) {
                for &succ in succs {
                    if reachable.insert(succ.to_string()) {
                        worklist.push(succ.to_string());
                    }
                }
            }
        }

        result.insert(name.clone(), reachable);
    }

    result
}
