//! Complete, hardware-independent dashboard configuration and validation.
//!
//! Parse through [`Config::parse`] or [`load`]. Validated models are exposed
//! immutably; raw values and parser messages must never be included in diagnostics.

mod bounded;
mod error;
mod lifecycle;
mod source;
mod theme;
mod widget;
mod wifi;

pub use error::{ConfigError, ErrorKind, Position};
pub use lifecycle::{
    ConfigOrigin, ConfigReader, ConfigStore, LoadedConfig, ReadError, Reload, load,
};
pub use source::{RequestAuth, Source, SourceKind};
pub use theme::{Color, Palette, Theme, ThemeMode};
pub use widget::{
    Aggregation, Binding, ColorRole, GaugeStyle, Placement, Range, Rect, Scale, Screen, Threshold,
    Widget, WidgetKind,
};
pub use wifi::{Authentication, Secret, WifiConfig};

use alloc::{collections::BTreeMap, format, string::String, vec::Vec};
use bounded::{List, Map};
use core::fmt;
use serde::Deserialize;

/// Maximum encoded document size, including comments, in bytes.
pub const MAX_CONFIG_BYTES: usize = 8192;
/// Allocation-free preflight limit, including whitespace/comment tokens.
pub const MAX_CONFIG_TOKENS: usize = 2048;
/// Maximum named HTTP sources.
pub const MAX_SOURCES: usize = 8;
/// Maximum screens.
pub const MAX_SCREENS: usize = 8;
/// Maximum widgets across all screens.
pub const MAX_WIDGETS: usize = 16;
/// Device-wide retained sample capacity, shared by all source types.
pub const MAX_HISTORY_POINTS: usize = 1536;
/// Maximum reserved discovered series, including latest-only bindings.
pub const MAX_TRACKED_SERIES: usize = 16;
/// Default history capacity for each graph series.
pub const DEFAULT_HISTORY_POINTS: u16 = 120;
/// Maximum retained samples in a single series.
pub const MAX_POINTS_PER_SERIES: u16 = 240;
/// Maximum matching series reserved by one graph.
pub const MAX_SERIES_PER_WIDGET: u8 = 12;
/// Configuration filename at the MicroSD root.
pub const CONFIG_FILENAME: &str = "graphdis.toml";
/// Embedded fallback selected by the build's `GRAPHDIS_CONFIG` setting.
pub const COMPILED_CONFIG: &str = include_str!(env!("GRAPHDIS_CONFIG_PATH"));

/// A shared history/latest-value reservation; references its canonical widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reservation {
    /// Index of the canonical widget's screen.
    pub screen: usize,
    /// Index of the canonical widget within that screen.
    pub widget: usize,
    /// Maximum discovered series sharing these sampling semantics.
    pub series: usize,
    /// Samples per series, zero for latest-only reservations.
    pub points_per_series: usize,
}

/// Fully validated dashboard configuration, including resolved palettes and budgets.
pub struct Config {
    wifi: Option<WifiConfig>,
    theme: Theme,
    sources: BTreeMap<String, Source>,
    screens: Vec<Screen>,
    rotation_secs: Option<u32>,
    touch_calibration: Option<[f64; 6]>,
    reservations: Vec<Reservation>,
    history_points: usize,
    tracked_series: usize,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("sources", &self.sources.len())
            .field("screens", &self.screens.len())
            .field("history_points", &self.history_points)
            .field("tracked_series", &self.tracked_series)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default = "version_one")]
    version: u16,
    wifi: Option<WifiConfig>,
    #[serde(default)]
    theme: theme::RawTheme,
    #[serde(default)]
    sources: Map<source::RawSource, MAX_SOURCES>,
    #[serde(default)]
    screens: List<widget::RawScreen, MAX_SCREENS>,
    #[serde(default)]
    display: Display,
    #[serde(default)]
    touch: Touch,
}

fn version_one() -> u16 {
    1
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Display {
    rotation_secs: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    calibration: Option<[f64; 6]>,
}

impl Config {
    /// Parse the entire document, resolving colors, references, layout, and budgets.
    ///
    /// Unknown fields are errors at every schema level. A dashboard must have at
    /// least one screen, though a setup screen may contain no widgets or sources.
    /// This checks configuration only, not network reachability or PromQL syntax.
    pub fn parse(document: &str) -> Result<Self, ConfigError> {
        if document.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::limit(
                ErrorKind::Limit,
                "document",
                document.len(),
                MAX_CONFIG_BYTES,
            ));
        }
        // toml first materializes tokens/events before serde collection limits
        // apply. Bound that work before allocating, including malformed input.
        use toml_parser::lexer::TokenKind;
        let mut tokens = 0usize;
        let mut depth = 0usize;
        for token in toml_parser::Source::new(document).lex() {
            tokens += 1;
            if tokens > MAX_CONFIG_TOKENS {
                return Err(ConfigError::limit(
                    ErrorKind::Limit,
                    "document.tokens",
                    tokens,
                    MAX_CONFIG_TOKENS,
                ));
            }
            match token.kind() {
                TokenKind::LeftSquareBracket | TokenKind::LeftCurlyBracket => {
                    depth += 1;
                    if depth > 16 {
                        return Err(ConfigError::limit(
                            ErrorKind::Limit,
                            "document.nesting",
                            depth,
                            16,
                        ));
                    }
                }
                TokenKind::RightSquareBracket | TokenKind::RightCurlyBracket => {
                    depth = depth.saturating_sub(1)
                }
                _ => {}
            }
        }
        // Conservative target-side allowance for token/event vectors, decoded
        // tables, deserialized fields, and allocator overhead. Keep this margin
        // in step with the pinned parser; free-heap logging supports board checks.
        #[cfg(feature = "firmware")]
        {
            let required = 8192 + tokens * 72 + document.len() * 4;
            let available = esp_alloc::HEAP.free();
            if required > available {
                return Err(ConfigError::limit(
                    ErrorKind::Limit,
                    "document.heap",
                    required,
                    available,
                ));
            }
        }
        let raw: RawConfig =
            toml::from_str(document).map_err(|e| ConfigError::syntax(e, document))?;
        if raw.version != 1 {
            return Err(ConfigError::new(
                ErrorKind::Value,
                "version",
                "unsupported configuration version; expected 1",
            ));
        }
        if let Some(wifi) = &raw.wifi {
            wifi.validate()?;
        }
        if let Some(c) = raw.touch.calibration
            && (c.iter().any(|v| !v.is_finite() || v.abs() > 32768.0)
                || (c[0] * c[4] - c[1] * c[3]).abs() < 1e-9)
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                "touch.calibration",
                "expected six finite affine coefficients with a nonzero transform",
            ));
        }
        let theme = raw.theme.resolve()?;
        let mut sources = BTreeMap::new();
        for (index, (name, source)) in raw.sources.0.into_iter().enumerate() {
            if !identifier(&name) {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &format!("sources[{index}]"),
                    "source identifier must be 1–32 ASCII letters, digits, underscores, or hyphens, starting with a letter or underscore",
                ));
            }
            sources.insert(name, source.resolve(index)?);
        }
        if raw.screens.0.is_empty() {
            return Err(ConfigError::new(
                ErrorKind::Value,
                "screens",
                "at least one screen is required",
            ));
        }
        if raw
            .display
            .rotation_secs
            .is_some_and(|secs| !(1..=86400).contains(&secs))
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                "display.rotation_secs",
                "rotation interval must be 1–86400 seconds, or omitted",
            ));
        }
        let mut screens = Vec::with_capacity(raw.screens.0.len());
        let mut count = 0;
        for (index, screen) in raw.screens.0.into_iter().enumerate() {
            let screen = screen.validate(&sources, index)?;
            count += screen.widgets.len();
            if count > MAX_WIDGETS {
                return Err(ConfigError::limit(
                    ErrorKind::Limit,
                    "screens.widgets",
                    count,
                    MAX_WIDGETS,
                ));
            }
            screens.push(screen);
        }
        let mut config = Self {
            wifi: raw.wifi,
            theme,
            sources,
            screens,
            rotation_secs: raw.display.rotation_secs,
            touch_calibration: raw.touch.calibration,
            reservations: Vec::new(),
            history_points: 0,
            tracked_series: 0,
        };
        config.reserve()?;
        Ok(config)
    }

    pub fn touch_calibration(&self) -> Option<[f64; 6]> {
        self.touch_calibration
    }
    /// Optional Wi-Fi credentials; absence leaves networking disabled.
    pub fn wifi(&self) -> Option<&WifiConfig> {
        self.wifi.as_ref()
    }
    /// Independently resolved palettes and selected mode.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }
    /// Named, validated sources. Source URLs and credentials must not be logged.
    pub fn sources(&self) -> &BTreeMap<String, Source> {
        &self.sources
    }
    /// Screens in navigation order, with resolved widget rectangles.
    pub fn screens(&self) -> &[Screen] {
        &self.screens
    }
    /// Optional timed-rotation interval, in seconds.
    pub fn rotation_secs(&self) -> Option<u32> {
        self.rotation_secs
    }
    /// Shared per-binding reservations; indices match Widget::reservation.
    pub fn reservations(&self) -> &[Reservation] {
        &self.reservations
    }
    /// Reserved samples across all screens, not just currently populated points.
    pub fn history_points(&self) -> usize {
        self.history_points
    }
    /// Reserved series, including latest-only widgets.
    pub fn tracked_series(&self) -> usize {
        self.tracked_series
    }

    fn reserve(&mut self) -> Result<(), ConfigError> {
        for screen in 0..self.screens.len() {
            for widget in 0..self.screens[screen].widgets.len() {
                let current = &self.screens[screen].widgets[widget];
                let existing = self.reservations.iter().position(|entry| {
                    let earlier = &self.screens[entry.screen].widgets[entry.widget];
                    same_sampling(current, earlier, self.sources[&current.source].kind())
                });
                let points = usize::from(current.history_points);
                let series = usize::from(current.max_series);
                let reservation = match existing {
                    Some(index) => {
                        let entry = &mut self.reservations[index];
                        entry.points_per_series = entry.points_per_series.max(points);
                        entry.series = entry.series.max(series);
                        index
                    }
                    None => {
                        self.reservations.push(Reservation {
                            screen,
                            widget,
                            series,
                            points_per_series: points,
                        });
                        self.reservations.len() - 1
                    }
                };
                self.screens[screen].widgets[widget].reservation = reservation;
            }
        }
        self.history_points = self
            .reservations
            .iter()
            .map(|r| r.series * r.points_per_series)
            .sum();
        self.tracked_series = self.reservations.iter().map(|r| r.series).sum();
        if self.history_points > MAX_HISTORY_POINTS {
            return Err(ConfigError::limit(
                ErrorKind::History,
                "screens.widgets.history_points",
                self.history_points,
                MAX_HISTORY_POINTS,
            ));
        }
        if self.tracked_series > MAX_TRACKED_SERIES {
            return Err(ConfigError::limit(
                ErrorKind::History,
                "screens.widgets.max_series",
                self.tracked_series,
                MAX_TRACKED_SERIES,
            ));
        }
        Ok(())
    }
}

fn same_sampling(a: &Widget, b: &Widget, kind: SourceKind) -> bool {
    a.source == b.source
        && a.binding == b.binding
        && a.aggregate == b.aggregate
        && (kind == SourceKind::Metrics || a.window_secs == b.window_secs)
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 32
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn text(value: &str, max: usize, empty: bool, field: &str) -> Result<(), ConfigError> {
    if (!empty && value.trim().is_empty())
        || value.len() > max
        || value.chars().any(char::is_control)
    {
        return Err(ConfigError::new(
            ErrorKind::Value,
            field,
            "text is empty, too long, or contains control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
