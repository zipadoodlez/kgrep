//! Peak memory reporting, so the memory claim stays measurable.
//!
//! Enabled only when `GRAPHGREP_PEAK_RSS` is set, which keeps the product
//! surface clean. `scripts/memcheck.sh` uses it to prove that peak memory does
//! not grow with repository size.

/// Peak resident set size of this process in kilobytes, if the platform
/// exposes it. Linux only; returns `None` elsewhere.
pub fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let number = rest.split_whitespace().next()?;
            return number.parse().ok();
        }
    }
    None
}

/// Print the peak resident set size to stderr when `GRAPHGREP_PEAK_RSS` is set.
pub fn report_if_requested() {
    if std::env::var_os("GRAPHGREP_PEAK_RSS").is_none() {
        return;
    }
    if let Some(kb) = peak_rss_kb() {
        eprintln!("peak_rss_kb: {kb}");
    }
}
