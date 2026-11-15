//! Test helper: checks that a function's work grows about linearly with its
//! input, so hostile input cannot make it quadratic.

use std::time::Duration;

const STACK_BYTES: usize = 256 << 20;

/// Asserts that `run(4 * n)` takes less than ten times `run(n)` (a quadratic
/// cost takes sixteen times), keeping the fastest of three runs of each.
pub fn assert_near_linear(what: &str, n: usize, run: impl Fn(usize) -> Duration + Sync) {
    // Deeply nested test inputs need more stack than a test thread has.
    let timed = |size| {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(STACK_BYTES)
                .spawn_scoped(scope, || run(size))
                .expect("spawn a thread for the scaling run")
                .join()
                .expect("the scaling run does not panic")
        })
    };
    let fastest = |size| (0..3).map(|_| timed(size)).min().unwrap_or_default();
    let small = fastest(n);
    let large = fastest(4 * n);
    assert!(
        large < small * 10 + Duration::from_millis(20),
        "{what}: {n} took {small:?}, {} took {large:?}",
        4 * n
    );
}
