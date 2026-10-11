use super::extract::{Call, Callee, Instructions};

/// Instructions identified per callee. Calls whose role follows from their
/// own shape (listeners, embedded templates, `@for`, `@defer`) and update
/// bindings (resolved through the selected slot) are not listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    ElementStart,
    ElementEnd,
    Element,
    ContainerStart,
    ContainerEnd,
    Container,
    Text,
    Pipe,
    Projection,
    Advance,
}

fn ivy_role(name: &str) -> Option<Role> {
    Some(match name.strip_prefix("dom").unwrap_or(name) {
        "elementStart" | "ElementStart" => Role::ElementStart,
        "elementEnd" | "ElementEnd" => Role::ElementEnd,
        "element" | "Element" => Role::Element,
        "elementContainerStart" | "ElementContainerStart" => Role::ContainerStart,
        "elementContainerEnd" | "ElementContainerEnd" => Role::ContainerEnd,
        "elementContainer" | "ElementContainer" => Role::Container,
        "text" => Role::Text,
        "pipe" => Role::Pipe,
        "projection" => Role::Projection,
        "advance" => Role::Advance,
        _ => return None,
    })
}

#[derive(Default, Clone)]
struct Stats {
    /// Creation calls per template function, indexed like `views`.
    per_view: Vec<usize>,
    total: usize,
    zero_arity: usize,
    /// `(slot, "string", ...)`
    slot_string: usize,
    /// `(slot)`
    slot_only: usize,
    /// `(slot, int)`
    slot_int: usize,
    max_arity: usize,
    update_calls: usize,
    all_tag_like: bool,
    free_text: bool,
    pipe_evidence: bool,
}

impl Stats {
    const fn used(&self) -> bool {
        self.total > 0
    }

    const fn only_zero_arity(&self) -> bool {
        self.used() && self.zero_arity == self.total && self.update_calls == 0
    }

    const fn only_slot_string(&self) -> bool {
        self.used() && self.slot_string == self.total
    }
}

fn is_tag_like(s: &str) -> bool {
    let s = s
        .strip_prefix(":svg:")
        .or_else(|| s.strip_prefix(":math:"))
        .unwrap_or(s);
    s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Infers which callee implements which instruction across every template
/// function of the bundle. Minified builds rename the `ɵɵ*` functions, so
/// roles are recovered from call shapes: `elementEnd` is the most frequent
/// zero-argument creation call, `elementStart` is the `(slot, "tag", ...)`
/// call it balances in every view, and so on.
pub(super) fn infer_roles(callees: &[Callee], views: &[Instructions<'_>]) -> Vec<Option<Role>> {
    let mut stats = vec![
        Stats {
            per_view: vec![0; views.len()],
            all_tag_like: true,
            ..Stats::default()
        };
        callees.len()
    ];
    for (view_index, view) in views.iter().enumerate() {
        for call in &view.create {
            let s = &mut stats[call.callee_index];
            s.per_view[view_index] += 1;
            observe_create(s, call, view);
        }
        for call in &view.update {
            stats[call.callee_index].update_calls += 1;
        }
    }

    let mut roles: Vec<Option<Role>> = callees.iter().map(|c| c.ivy_name().and_then(ivy_role)).collect();
    let unnamed: Vec<usize> = (0..callees.len())
        .filter(|&i| callees[i].ivy_name().is_none())
        .collect();
    let balanced = |opener: usize, closer: usize| stats[opener].per_view == stats[closer].per_view;
    let most_used = |candidates: Vec<usize>| candidates.into_iter().max_by_key(|&i| (stats[i].total, usize::MAX - i));

    let zero_arity: Vec<usize> = unnamed
        .iter()
        .copied()
        .filter(|&i| stats[i].only_zero_arity())
        .collect();
    if let Some(end) = most_used(zero_arity.clone()) {
        roles[end] = Some(Role::ElementEnd);
        let openers = unnamed
            .iter()
            .copied()
            .filter(|&i| stats[i].only_slot_string() && balanced(i, end))
            .collect();
        if let Some(start) = most_used(openers) {
            roles[start] = Some(Role::ElementStart);
        }
    }
    'containers: for &end in &zero_arity {
        if roles[end].is_some() {
            continue;
        }
        for &start in &unnamed {
            let s = &stats[start];
            if roles[start].is_none() && s.used() && s.slot_only + s.slot_int == s.total && balanced(start, end) {
                roles[start] = Some(Role::ContainerStart);
                roles[end] = Some(Role::ContainerEnd);
                break 'containers;
            }
        }
    }
    for &i in &unnamed {
        let s = &stats[i];
        if roles[i].is_some() || !s.used() || s.max_arity > 4 {
            continue;
        }
        roles[i] = if s.only_slot_string() && s.pipe_evidence && s.max_arity == 2 {
            Some(Role::Pipe)
        } else if s.slot_string + s.slot_only == s.total
            && s.max_arity <= 2
            && (s.slot_only > 0 || s.free_text || !s.all_tag_like)
        {
            Some(Role::Text)
        } else if s.only_slot_string() && s.all_tag_like {
            Some(Role::Element)
        } else {
            None
        };
    }

    let mut advance_shaped = vec![0usize; callees.len()];
    let mut other_shaped = vec![false; callees.len()];
    for view in views {
        for call in &view.update {
            if call.args.is_empty() || (call.args.len() == 1 && call.int(0).is_some()) {
                advance_shaped[call.callee_index] += 1;
            } else {
                other_shaped[call.callee_index] = true;
            }
        }
    }
    let advance = unnamed
        .iter()
        .copied()
        .filter(|&i| !stats[i].used() && !other_shaped[i] && advance_shaped[i] > 0)
        .max_by_key(|&i| (advance_shaped[i], usize::MAX - i));
    if let Some(advance) = advance {
        roles[advance] = Some(Role::Advance);
    }
    roles
}

fn observe_create(s: &mut Stats, call: &Call<'_>, view: &Instructions<'_>) {
    let arity = call.args.len();
    s.total += 1;
    s.max_arity = s.max_arity.max(arity);
    match (call.int(0), call.string(1)) {
        _ if arity == 0 => s.zero_arity += 1,
        (Some(slot), Some(text)) => {
            s.slot_string += 1;
            s.all_tag_like &= is_tag_like(text);
            s.free_text |= text.contains(char::is_whitespace);
            s.pipe_evidence |= view.pipe_slots.contains(&slot);
        }
        (Some(_), None) if arity == 1 => s.slot_only += 1,
        (Some(_), None) if arity == 2 && call.int(1).is_some() => s.slot_int += 1,
        _ => {}
    }
}
