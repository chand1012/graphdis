//! Allocation-free dashboard drawing. Coordinates are logical 320 × 240 pixels.
//!
//! The application owns history and supplies borrowed snapshots. Drawing never
//! fetches data or changes source state. Hardware errors propagate to the caller.
#[cfg(feature = "firmware")]
pub mod board;
mod cache;
pub mod calibration_files;
pub mod controls;
pub use cache::{DrawStats, Renderer};
mod format;
mod navigation;
mod render;
pub mod touch;

pub use navigation::{Button, Navigation};
pub use render::{diagnostic, render};

/// A timestamped reading. Timestamps and `now_ms` must share a monotonic epoch.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub timestamp_ms: u64,
    pub value: f64,
}

/// A stable series slot: preserve ordering between frames for stable colors.
#[derive(Clone, Copy)]
pub struct Series<'a> {
    /// Stable palette slot independent of response ordering or missing series.
    pub color_slot: usize,
    pub label: &'a str,
    /// Canonical length-framed metric identity, used for legend placeholders.
    pub identity: &'a str,
    /// Chronologically ordered samples. NaN/infinity create line gaps.
    pub samples: &'a [Sample],
}

/// Source state independently of its retained readings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Loading,
    Ready,
    NoData,
    Error,
    /// Age of the last successful update, supplied by the data owner.
    Stale {
        age_secs: u32,
    },
}

/// Snapshot for one widget. The caller must supply no more than its configured
/// max_series/history_points; rendering also caps reads to these limits.
#[derive(Clone, Copy, Default)]
pub struct WidgetData<'a> {
    pub state: State,
    /// Optional finite extrema from a bounded GaugeRange tracker. Ignored for
    /// graphs, whose range is derived from their visible timestamped samples.
    pub observed_range: Option<(f64, f64)>,
    pub series: &'a [Series<'a>],
}

/// Constant-size gauge extrema since startup or configuration reload. Own one
/// per reservation; reset on reload. It retains no sample history (24 bytes on
/// ESP32). The range expands as values arrive, avoiding sample-by-sample jitter.
#[derive(Default)]
pub struct GaugeRange {
    bounds: Option<(f64, f64)>,
}
impl GaugeRange {
    pub fn observe(&mut self, value: f64) {
        if value.is_finite() {
            self.bounds = Some(
                self.bounds
                    .map_or((value, value), |(a, b)| (a.min(value), b.max(value))),
            );
        }
    }
    pub fn bounds(&self) -> Option<(f64, f64)> {
        self.bounds
    }
    pub fn reset(&mut self) {
        self.bounds = None;
    }
}

/// Lookup by Config reservation, so identical bindings can share storage.
/// Missing entries render as Loading. This trait does not require allocation.
pub trait Data {
    fn with_widget<R>(&self, reservation: usize, draw: impl FnOnce(WidgetData<'_>) -> R) -> R;
}

/// Initial state before a polling/parser service has produced readings.
pub struct Loading;
impl Data for Loading {
    fn with_widget<R>(&self, _: usize, draw: impl FnOnce(WidgetData<'_>) -> R) -> R {
        draw(WidgetData::default())
    }
}

#[cfg(test)]
mod tests;
