/// Key for member-expression bindings. `this.foo` and `obj.foo` are tracked
/// separately because the receiver semantics differ: `this` is implicit and
/// resolves at call time, while `obj` is a concrete identifier.
#[derive(Clone, Eq, Hash, PartialEq, Debug)]
pub(super) enum MemberKey {
    This(String),
    Bound(String, String),
}
