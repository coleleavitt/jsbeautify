/// `eprintln!` that drops a line stderr cannot take, as in the library: a log
/// piped into a reader that exits early must not end the run.
macro_rules! eprintln {
    ($($arg:tt)*) => {
        jsbeautify::log_line(format_args!($($arg)*))
    };
}

/// `println!` / `print!` for the result on stdout. When stdout is a pipe
/// whose reader has exited (`jsbeautify a.js | head`), the rest of the
/// result has nowhere to go: exit quietly with 141, the status of a process
/// a `SIGPIPE` ends, instead of panicking (which aborts in release builds).
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::app::stdout(format_args!("{}\n", format_args!($($arg)*)))
    };
}
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::app::stdout(format_args!($($arg)*))
    };
}

#[path = "jsbeautify_app/mod.rs"]
mod app;

fn main() {
    app::main();
}
