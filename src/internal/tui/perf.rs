//! Lightweight performance telemetry for the TUI.
//!
//! Enabled by setting the `GIT_FORUM_PERF_LOG` environment variable to a file path.
//! When enabled, logs JSON lines with span timings. On TUI exit, prints a summary
//! (p50/p95/p99/max per span) to stderr. Zero cost when disabled.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;

const ENV_VAR: &str = "GIT_FORUM_PERF_LOG";

struct PerfLog {
    writer: BufWriter<File>,
}

impl PerfLog {
    fn from_env() -> Option<Self> {
        let path = std::env::var(ENV_VAR).ok()?;
        let path = PathBuf::from(path);
        let file = File::create(path).ok()?;
        Some(Self {
            writer: BufWriter::new(file),
        })
    }

    fn record(&mut self, span: &str, thread_id: Option<&str>, duration: Duration) {
        let duration_ms = duration.as_secs_f64() * 1000.0;
        let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let tid = thread_id.unwrap_or("");
        let _ = writeln!(
            self.writer,
            r#"{{"span":"{}","thread_id":"{}","duration_ms":{:.3},"ts":"{}"}}"#,
            span, tid, duration_ms, ts
        );
    }

    fn flush(&mut self) {
        let _ = self.writer.flush();
    }
}

struct PerfSummary {
    entries: HashMap<String, Vec<f64>>,
}

impl PerfSummary {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn record(&mut self, span: &str, duration: Duration) {
        let ms = duration.as_secs_f64() * 1000.0;
        self.entries.entry(span.to_string()).or_default().push(ms);
    }

    fn print_report(&self) {
        if self.entries.is_empty() {
            return;
        }
        eprintln!("\n--- TUI Performance Summary ---");
        eprintln!(
            "{:<30} {:>6} {:>8} {:>8} {:>8} {:>8}",
            "span", "count", "p50", "p95", "p99", "max"
        );
        let mut spans: Vec<&String> = self.entries.keys().collect();
        spans.sort();
        for span in spans {
            let vals = &self.entries[span];
            let mut sorted: Vec<f64> = vals.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = sorted.len();
            let p50 = sorted[n * 50 / 100];
            let p95 = sorted[(n * 95 / 100).min(n - 1)];
            let p99 = sorted[(n * 99 / 100).min(n - 1)];
            let max = sorted[n - 1];
            eprintln!(
                "{:<30} {:>6} {:>7.1}ms {:>7.1}ms {:>7.1}ms {:>7.1}ms",
                span, n, p50, p95, p99, max
            );
        }
        eprintln!("-------------------------------");
    }
}

/// Combined telemetry handle. Pass as `&mut Perf` through the event loop.
pub(crate) struct Perf {
    log: Option<PerfLog>,
    summary: PerfSummary,
    enabled: bool,
}

impl Perf {
    /// Create a new Perf instance. Checks `GIT_FORUM_PERF_LOG` env var.
    pub fn new() -> Self {
        let log = PerfLog::from_env();
        let enabled = log.is_some();
        Self {
            log,
            summary: PerfSummary::new(),
            enabled,
        }
    }

    /// Create a disabled instance (for tests).
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            log: None,
            summary: PerfSummary::new(),
            enabled: false,
        }
    }

    /// Record a span timing. No-op when disabled.
    pub fn record(&mut self, span: &str, thread_id: Option<&str>, duration: Duration) {
        if !self.enabled {
            return;
        }
        if let Some(ref mut log) = self.log {
            log.record(span, thread_id, duration);
        }
        self.summary.record(span, duration);
    }

    /// Flush log and print summary. Call on TUI exit.
    pub fn finish(mut self) {
        if !self.enabled {
            return;
        }
        if let Some(ref mut log) = self.log {
            log.flush();
        }
        self.summary.print_report();
    }
}
