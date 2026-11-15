//! The scope tree numbered in preorder, for subtree tests in `O(1)`.

use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;

/// The scope tree in preorder. A subtree is the position range
/// `[position(scope), last[position(scope)]]`.
pub struct ScopeTree {
    position_of: Vec<usize>,
    last: Vec<usize>,
}

impl ScopeTree {
    #[must_use]
    pub fn new(scoping: &Scoping) -> Self {
        let len = scoping.scopes_len();
        let mut children: Vec<Vec<ScopeId>> = vec![Vec::new(); len];
        let mut roots = Vec::new();
        for scope in scoping.scope_descendants_from_root() {
            match scoping.scope_parent_id(scope) {
                Some(parent) => children[parent.index()].push(scope),
                None => roots.push(scope),
            }
        }
        let mut position_of = vec![0; len];
        let mut last = vec![0; len];
        let mut visited = 0;
        // Explicit stack: nesting depth comes from the input.
        let mut stack: Vec<(ScopeId, usize)> = Vec::new();
        for root in roots {
            position_of[root.index()] = visited;
            visited += 1;
            stack.push((root, 0));
            while let Some((scope, next_child)) = stack.last_mut() {
                if let Some(&child) = children[scope.index()].get(*next_child) {
                    *next_child += 1;
                    position_of[child.index()] = visited;
                    visited += 1;
                    stack.push((child, 0));
                } else {
                    last[position_of[scope.index()]] = visited - 1;
                    stack.pop();
                }
            }
        }
        Self { position_of, last }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.last.len()
    }

    #[must_use]
    pub fn position(&self, scope: ScopeId) -> usize {
        self.position_of[scope.index()]
    }

    /// The last position in the subtree of the scope at `position`.
    #[must_use]
    pub fn last(&self, position: usize) -> usize {
        self.last[position]
    }

    /// The positions of `scope` and its descendants.
    #[must_use]
    pub fn subtree(&self, scope: ScopeId) -> std::ops::RangeInclusive<usize> {
        let position = self.position(scope);
        position..=self.last(position)
    }
}
