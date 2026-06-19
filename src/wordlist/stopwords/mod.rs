//! Stopword lists recovered from jxscout-pro-v2 binary.
//!
//! Three lists are combined by [`super::filter::is_stopword`]:
//!   1. English stopwords (the, and, for, ...)
//!   2. JS keyword / runtime names (return, typeof, console, ...)
//!   3. Common file extensions (ts, jsx, woff2, ...)

mod english;
mod extensions;
mod js;

pub use english::ENGLISH;
pub use extensions::EXTENSIONS;
pub use js::JS_KEYWORDS;
