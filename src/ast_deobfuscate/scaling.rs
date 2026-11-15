//! Test helper: checks that a function's work grows about linearly with its
//! input, so hostile input cannot make it quadratic.

use std::time::Duration;

/// Asserts that `run(4 * n)` takes less than ten times `run(n)` (a quadratic
/// cost takes sixteen times), keeping the fastest of three runs of each.
pub fn assert_near_linear(what: &str, n: usize, run: impl Fn(usize) -> Duration) {
    let fastest = |size| (0..3).map(|_| run(size)).min().unwrap_or_default();
    let small = fastest(n);
    let large = fastest(4 * n);
    assert!(
        large < small * 10 + Duration::from_millis(20),
        "{what}: {n} took {small:?}, {} took {large:?}",
        4 * n
    );
}
