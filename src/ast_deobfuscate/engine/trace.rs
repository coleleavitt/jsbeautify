use std::sync::OnceLock;
use std::time::Instant;

/// Resident set size in `MiB` (Linux). Backs the `JSB_MEM` memory-profiling
/// checkpoints below — `/proc/self/statm` field 2 is resident pages.
#[cfg(target_os = "linux")]
fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()))
        .map_or(0, |pages| pages * 4096 / (1024 * 1024))
}
#[cfg(not(target_os = "linux"))]
fn rss_mb() -> u64 {
    0
}

/// Emit an RSS checkpoint when `JSB_MEM` is set, to localise the pipeline's
/// peak-memory phase. No-op (and no syscall) in normal runs.
pub(super) fn mem_ck(label: &str) {
    if std::env::var_os("JSB_MEM").is_some() {
        eprintln!("[MEM] {:>5} MB  {label}", rss_mb());
    }
}

static TRACE_DEOB: OnceLock<bool> = OnceLock::new();
static TRACE_AST: OnceLock<bool> = OnceLock::new();

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

pub fn deob_trace_enabled() -> bool {
    *TRACE_DEOB.get_or_init(|| env_flag("JSBEAUTIFY_TRACE_DEOB") || env_flag("JSB_TRACE_DEOB"))
}

pub fn ast_trace_enabled() -> bool {
    *TRACE_AST.get_or_init(|| env_flag("JSBEAUTIFY_TRACE_AST") || env_flag("JSB_TRACE_AST"))
}

pub struct TraceSpan {
    label: &'static str,
    start: Instant,
    start_rss: u64,
    enabled: bool,
}

impl TraceSpan {
    #[must_use]
    pub(crate) fn new(label: &'static str) -> Self {
        let enabled = deob_trace_enabled();
        let start_rss = if enabled { rss_mb() } else { 0 };
        if enabled {
            eprintln!("[TRACE] START {label} rss={start_rss} MB");
        }
        Self {
            label,
            start: Instant::now(),
            start_rss,
            enabled,
        }
    }
}

impl Drop for TraceSpan {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        let end_rss = rss_mb();
        let rss_delta = i64::try_from(end_rss).unwrap_or(i64::MAX) - i64::try_from(self.start_rss).unwrap_or(i64::MAX);
        eprintln!(
            "[TRACE] END   {} elapsed={:.3}s rss={} MB delta={:+} MB",
            self.label,
            self.start.elapsed().as_secs_f64(),
            end_rss,
            rss_delta
        );
    }
}
