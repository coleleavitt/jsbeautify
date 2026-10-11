//! Runs parsing and AST work on a thread whose stack is sized from the input.
//!
//! Oxc's parser, semantic builder and traversals, our passes and the layout
//! all recurse once per nesting level, and nesting depth never exceeds the
//! input length. A stack of `BASE_STACK_BYTES + len * STACK_BYTES_PER_INPUT_BYTE`
//! therefore fits any input. The reservation is virtual: pages that deep
//! nesting never touches cost no memory.

use std::cell::Cell;
use std::sync::{Mutex, PoisonError};

const BASE_STACK_BYTES: usize = 16 << 20;
// Measured on the whole `-d` pipeline (2026-10-11, release): stack per nesting
// level is constant — brackets 1.87 KB, calls 1.70 KB, arrows 1.19 KB, ternary
// 1.11 KB, `+`/`&&` 0.74 KB, else-if 0.55 KB, member 0.40 KB. Per input byte the
// worst shape is brackets (`[` and `]` per level), about 0.94 KB, so 2 KiB/byte
// leaves a 2x margin. Debug frames are several times larger. Re-measure by
// binary-searching the deepest input that passes at a fixed stack size.
const STACK_BYTES_PER_INPUT_BYTE: usize = if cfg!(debug_assertions) { 16 << 10 } else { 2 << 10 };
const MIN_STACK_BYTES: usize = 8 << 20;

thread_local! {
    /// Stack size requested for the current worker; nested calls whose input
    /// needs no more than this run inline.
    static REQUESTED_STACK_BYTES: Cell<usize> = const { Cell::new(0) };
}

const fn stack_bytes_for(input_len: usize) -> usize {
    input_len
        .saturating_mul(STACK_BYTES_PER_INPUT_BYTE)
        .saturating_add(BASE_STACK_BYTES)
}

/// Run `work` on a thread with enough stack for an input of `input_len` bytes,
/// or inline when the current thread already is such a worker. Panics in
/// `work` propagate to the caller.
///
/// When the OS refuses the reservation, the size is halved down to a floor and
/// a warning is printed, since very deep input may then overflow the stack.
///
/// # Panics
/// Panics when no worker thread can be spawned even at the floor size.
pub fn run_on_sized_stack<T: Send>(input_len: usize, work: impl FnOnce() -> T + Send) -> T {
    let requested = stack_bytes_for(input_len);
    if REQUESTED_STACK_BYTES.get() >= requested {
        return work();
    }
    let work = Mutex::new(Some(work));
    let run = || {
        REQUESTED_STACK_BYTES.set(requested);
        let work = work.lock().unwrap_or_else(PoisonError::into_inner).take();
        work.expect("work runs on exactly one spawned thread")()
    };
    std::thread::scope(|scope| {
        let mut stack_bytes = requested;
        loop {
            match std::thread::Builder::new()
                .name("js-beautify-worker".into())
                .stack_size(stack_bytes)
                .spawn_scoped(scope, run)
            {
                Ok(worker) => {
                    if stack_bytes < requested {
                        eprintln!(
                            "warning: could not reserve a {} MiB stack for this input; using {} MiB, \
                             so very deeply nested input may overflow it",
                            requested >> 20,
                            stack_bytes >> 20
                        );
                    }
                    return worker
                        .join()
                        .unwrap_or_else(|payload| std::panic::resume_unwind(payload));
                }
                Err(_) if stack_bytes > MIN_STACK_BYTES => stack_bytes = (stack_bytes / 2).max(MIN_STACK_BYTES),
                Err(error) => panic!("cannot spawn a worker thread with a {stack_bytes}-byte stack: {error}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_grows_linearly_with_input_and_saturates() {
        assert_eq!(stack_bytes_for(0), BASE_STACK_BYTES);
        assert_eq!(
            stack_bytes_for(1000) - stack_bytes_for(0),
            1000 * STACK_BYTES_PER_INPUT_BYTE
        );
        assert_eq!(stack_bytes_for(usize::MAX), usize::MAX);
    }
}
