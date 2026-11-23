//! Serial timing logs for startup and collection profiling.
//!
//! Stamps use the esp-hal monotonic counter, which starts during `esp_hal::init`,
//! so they exclude ROM and second-stage bootloader time. Lines are short because
//! esp-println writes UART0 synchronously: each 60-character line costs roughly
//! 5 ms at 115200 baud, and that cost is included in the spans around it.
//! Never pass credentials, URLs, headers, or raw configuration text.

use core::fmt;
use esp_hal::time::{Duration, Instant};

/// Milliseconds with microsecond precision, for compact log output.
#[derive(Clone, Copy)]
pub struct Ms(pub Duration);
impl fmt::Display for Ms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let us = self.0.as_micros();
        write!(f, "{}.{:03}ms", us / 1000, us % 1000)
    }
}

/// Time since `esp_hal::init` started the monotonic counter.
pub fn uptime() -> Duration {
    Instant::now().duration_since_epoch()
}

/// Print one timestamped line. Prefer the [`timing!`](crate::timing!) macro.
pub fn mark(args: fmt::Arguments<'_>) {
    esp_println::println!("[T {:>10}] {}", Ms(uptime()), args);
}

/// Print a timestamped line, formatted like `println!`.
#[macro_export]
macro_rules! timing {
    ($($arg:tt)*) => {
        $crate::timing::mark(format_args!($($arg)*))
    };
}

/// A measured region. Logs its start when created and its duration when finished.
#[must_use = "finish the span to log its duration"]
pub struct Span {
    label: &'static str,
    start: Instant,
}

impl Span {
    /// Log `label` as started and begin measuring.
    pub fn start(label: &'static str) -> Self {
        mark(format_args!("{label}: start"));
        Self::quiet(label)
    }
    /// Begin measuring without a start line, for short or frequent regions.
    pub fn quiet(label: &'static str) -> Self {
        Self {
            label,
            start: Instant::now(),
        }
    }
    /// Elapsed time so far, without finishing.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
    /// Log the duration and return it.
    pub fn done(self) -> Duration {
        let elapsed = self.start.elapsed();
        mark(format_args!("{}: done in {}", self.label, Ms(elapsed)));
        elapsed
    }
    /// Log the duration with extra detail and return it.
    pub fn done_with(self, detail: fmt::Arguments<'_>) -> Duration {
        let elapsed = self.start.elapsed();
        mark(format_args!(
            "{}: done in {} ({})",
            self.label,
            Ms(elapsed),
            detail
        ));
        elapsed
    }
}
