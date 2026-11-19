use super::{
    Color, ConfigError, ErrorKind, Palette, Source, SourceKind, ThemeMode,
    bounded::{List, Map},
};
use alloc::{collections::BTreeMap, format, string::String, vec::Vec};
use serde::Deserialize;

/// Widget visualization.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    /// Multi-series line graph with local or server-provided history.
    Graph,
    /// Single-series bar or dial.
    Gauge,
    /// Single-series compact value.
    Number,
}

/// Gauge geometry; bars are the default.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GaugeStyle {
    /// Horizontal progress bar.
    #[default]
    Bar,
    /// Semicircular progress arc.
    Dial,
}

/// Numeric prefix system, independent of the unit label.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// SI prefixes, factors of 1,000.
    #[default]
    Decimal,
    /// IEC prefixes, factors of 1,024.
    Binary,
    /// No prefix scaling, useful for percentages and temperatures.
    None,
}

/// Collapse matched exposition samples to one bounded output series.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    #[default]
    None,
    Mean,
}

/// Theme-relative threshold color role; literal overrides remain hex strings.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorRole {
    /// Theme accent.
    Accent,
    /// Theme success.
    Success,
    /// Theme warning.
    #[default]
    Warning,
    /// Theme error.
    Error,
}

/// A lower-inclusive threshold in actual, unrounded metric units.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    /// This threshold applies when the actual value is at least this boundary.
    pub value: f64,
    /// Semantic role, warning by default.
    #[serde(default)]
    pub role: ColorRole,
    /// Optional dark-mode literal override.
    pub dark: Option<Color>,
    /// Optional light-mode literal override.
    pub light: Option<Color>,
}

impl Threshold {
    /// Resolve an independent mode override against a semantic palette role.
    pub fn resolve(&self, mode: ThemeMode, palette: &Palette) -> Color {
        let custom = match mode {
            ThemeMode::Dark => self.dark,
            ThemeMode::Light => self.light,
        };
        custom.unwrap_or(match self.role {
            ColorRole::Accent => palette.accent,
            ColorRole::Success => palette.success,
            ColorRole::Warning => palette.warning,
            ColorRole::Error => palette.error,
        })
    }
}

/// Optional finite axis/gauge bounds. A single bound leaves the other automatic.
#[derive(Clone, Copy, Debug, Default)]
pub struct Range {
    /// Explicit lower bound.
    pub min: Option<f64>,
    /// Explicit upper bound.
    pub max: Option<f64>,
}

/// Resolved widget rectangle, including border, in logical landscape pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left coordinate.
    pub x: u16,
    /// Top coordinate.
    pub y: u16,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
}

/// Grid location and spans; indices are zero-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Row index.
    pub row: u8,
    /// Column index.
    pub column: u8,
    /// Occupied row count.
    pub row_span: u8,
    /// Occupied column count.
    pub column_span: u8,
}

/// Source-independent sample selection, preserving exact matching semantics.
#[derive(PartialEq, Eq)]
pub enum Binding {
    /// Metric name and exact label equality filters.
    Metric {
        /// Exposition metric name.
        name: String,
        /// Label filters, in canonical key order.
        labels: BTreeMap<String, String>,
    },
    /// Server-evaluated PromQL text.
    Query(String),
}

/// Validated widget. Accessed immutably through the owning Config.
pub struct Widget {
    /// Visualization kind.
    pub kind: WidgetKind,
    /// User-visible title.
    pub title: String,
    /// Optional legend text; {{label}} placeholders use the original series labels.
    pub legend: Option<String>,
    /// Name of a validated source.
    pub source: String,
    /// Validated source-compatible selection.
    pub binding: Binding,
    /// Grid placement.
    pub placement: Placement,
    /// Calculated outer bounds.
    pub rect: Rect,
    /// Unit suffix, up to 16 UTF-8 bytes.
    pub unit: String,
    /// Prefix system.
    pub scale: Scale,
    /// Requested significant digits, 1–6; renderer may reduce to fit.
    pub precision: u8,
    /// Graph history per series; zero for latest-only widgets.
    pub history_points: u16,
    /// Reserved maximum number of matching series; latest-only widgets require one.
    pub max_series: u8,
    /// Optional mean across matching samples, calculated per complete scrape.
    pub aggregate: Aggregation,
    /// Graph time window in seconds; zero for latest-only widgets.
    pub window_secs: u32,
    /// Graph or gauge range; unused for numeric widgets.
    pub range: Range,
    /// Gauge style; ignored for non-gauges.
    pub style: GaugeStyle,
    /// Increasing lower-inclusive gauge thresholds, at most eight.
    pub thresholds: Vec<Threshold>,
    /// Index into Config::reservations, shared by equivalent bindings.
    pub reservation: usize,
}

impl Widget {
    /// Gauge color based on the unrounded value; invalid values have no fill.
    pub fn gauge_color(&self, value: f64, mode: ThemeMode, palette: &Palette) -> Option<Color> {
        if self.kind != WidgetKind::Gauge || !value.is_finite() {
            return None;
        }
        Some(
            self.thresholds
                .iter()
                .rev()
                .find(|t| value >= t.value)
                .map_or(palette.accent, |t| t.resolve(mode, palette)),
        )
    }
}

/// Validated screen, holding at most sixteen widgets before the device-wide limit.
pub struct Screen {
    /// Screen name shown in navigation.
    pub title: String,
    /// Grid row count, 1–4.
    pub rows: u8,
    /// Grid column count, 1–4.
    pub columns: u8,
    /// Widgets in configuration order.
    pub widgets: Vec<Widget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawScreen {
    title: String,
    #[serde(default = "one")]
    rows: u8,
    #[serde(default = "one")]
    columns: u8,
    #[serde(default)]
    widgets: List<RawWidget, { super::MAX_WIDGETS }>,
}

fn one() -> u8 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWidget {
    #[serde(rename = "type")]
    kind: WidgetKind,
    title: String,
    source: String,
    legend: Option<String>,
    metric: Option<String>,
    #[serde(default)]
    labels: Map<String, 8>,
    query: Option<String>,
    #[serde(default)]
    row: u8,
    #[serde(default)]
    column: u8,
    #[serde(default = "one")]
    row_span: u8,
    #[serde(default = "one")]
    column_span: u8,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    scale: Scale,
    precision: Option<u8>,
    history_points: Option<u16>,
    max_series: Option<u8>,
    #[serde(default)]
    aggregate: Aggregation,
    window_secs: Option<u32>,
    min: Option<f64>,
    max: Option<f64>,
    style: Option<GaugeStyle>,
    thresholds: Option<List<RawThreshold, 8>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawThreshold {
    value: f64,
    #[serde(default)]
    role: ColorRole,
    dark: Option<String>,
    light: Option<String>,
}

impl RawScreen {
    pub fn validate(
        self,
        sources: &BTreeMap<String, Source>,
        index: usize,
    ) -> Result<Screen, ConfigError> {
        let field = format!("screens[{index}]");
        super::text(&self.title, 64, false, &format!("{field}.title"))?;
        if !(1..=4).contains(&self.rows) || !(1..=4).contains(&self.columns) {
            return Err(ConfigError::new(
                ErrorKind::Layout,
                &field,
                "grid rows and columns must be 1–4",
            ));
        }
        let mut occupied = [[false; 4]; 4];
        let mut widgets = Vec::with_capacity(self.widgets.0.len());
        for (widget_index, raw) in self.widgets.0.into_iter().enumerate() {
            let path = format!("{field}.widgets[{widget_index}]");
            let placement = Placement {
                row: raw.row,
                column: raw.column,
                row_span: raw.row_span,
                column_span: raw.column_span,
            };
            if raw.row_span == 0
                || raw.column_span == 0
                || u16::from(raw.row) + u16::from(raw.row_span) > u16::from(self.rows)
                || u16::from(raw.column) + u16::from(raw.column_span) > u16::from(self.columns)
            {
                return Err(ConfigError::new(
                    ErrorKind::Layout,
                    &path,
                    "widget spans must be positive and fit the screen grid",
                ));
            }
            for row in occupied
                .iter_mut()
                .skip(raw.row as usize)
                .take(raw.row_span as usize)
            {
                for cell in row
                    .iter_mut()
                    .skip(raw.column as usize)
                    .take(raw.column_span as usize)
                {
                    if *cell {
                        return Err(ConfigError::new(
                            ErrorKind::Layout,
                            &path,
                            "widget overlaps another widget",
                        ));
                    }
                    *cell = true;
                }
            }
            let (x, width) = track(312, self.columns, raw.column, raw.column_span);
            let (y, height) = track(192, self.rows, raw.row, raw.row_span);
            let rect = Rect {
                x: 4 + x,
                y: 4 + y,
                width,
                height,
            };
            widgets.push(raw.validate(sources, &path, placement, rect)?);
        }
        Ok(Screen {
            title: self.title,
            rows: self.rows,
            columns: self.columns,
            widgets,
        })
    }
}

fn track(total: u16, count: u8, start: u8, span: u8) -> (u16, u16) {
    let n = u16::from(count);
    let start = u16::from(start);
    let span = u16::from(span);
    let usable = total - 4 * (n - 1);
    let base = usable / n;
    let rem = usable % n;
    let offset = start * (base + 4) + start.min(rem);
    let width = span * base + (start + span).min(rem) - start.min(rem) + 4 * (span - 1);
    (offset, width)
}

impl RawWidget {
    fn validate(
        self,
        sources: &BTreeMap<String, Source>,
        field: &str,
        placement: Placement,
        rect: Rect,
    ) -> Result<Widget, ConfigError> {
        super::text(&self.title, 64, false, &format!("{field}.title"))?;
        super::text(&self.unit, 16, true, &format!("{field}.unit"))?;
        if !self
            .unit
            .chars()
            .all(|c| matches!(u32::from(c), 0x20..=0x7e | 0xa0..=0xff))
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                &format!("{field}.unit"),
                "unit must use glyphs supported by the Latin-1 fonts",
            ));
        }
        if let Some(legend) = &self.legend {
            super::text(legend, 128, false, &format!("{field}.legend"))?;
            if self.kind != WidgetKind::Graph {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    field,
                    "legend requires a graph",
                ));
            }
            let mut rest = legend.as_str();
            while let Some(start) = rest.find("{{") {
                rest = &rest[start + 2..];
                let Some(end) = rest.find("}}") else {
                    return Err(ConfigError::new(
                        ErrorKind::Value,
                        field,
                        "unclosed legend placeholder",
                    ));
                };
                if !prom_name(&rest[..end], false) {
                    return Err(ConfigError::new(
                        ErrorKind::Value,
                        field,
                        "invalid legend label name",
                    ));
                }
                rest = &rest[end + 2..];
            }
        }
        let source = sources.get(&self.source).ok_or_else(|| {
            ConfigError::new(
                ErrorKind::Reference,
                &format!("{field}.source"),
                "source name does not exist",
            )
        })?;
        let binding = match source.kind() {
            SourceKind::Metrics => {
                if self.query.is_some() {
                    return Err(ConfigError::new(
                        ErrorKind::Reference,
                        field,
                        "direct metrics require a metric selector, not a query",
                    ));
                }
                let name = self.metric.ok_or_else(|| {
                    ConfigError::new(ErrorKind::Reference, field, "direct metrics require metric")
                })?;
                if !prom_name(&name, true) || name.len() > 128 {
                    return Err(ConfigError::new(
                        ErrorKind::Value,
                        &format!("{field}.metric"),
                        "invalid metric name",
                    ));
                }
                for (index, (name, value)) in self.labels.0.iter().enumerate() {
                    if !prom_name(name, false) || name.len() > 64 || value.len() > 128 {
                        return Err(ConfigError::new(
                            ErrorKind::Value,
                            &format!("{field}.labels[{index}]"),
                            "invalid label name or excessive label length",
                        ));
                    }
                }
                Binding::Metric {
                    name,
                    labels: self.labels.0,
                }
            }
            SourceKind::Prometheus => {
                if self.metric.is_some() || !self.labels.0.is_empty() {
                    return Err(ConfigError::new(
                        ErrorKind::Reference,
                        field,
                        "Prometheus API sources require query without metric or labels",
                    ));
                }
                let query = self.query.ok_or_else(|| {
                    ConfigError::new(
                        ErrorKind::Reference,
                        field,
                        "Prometheus API sources require query",
                    )
                })?;
                if query.trim().is_empty()
                    || query.len() > 512
                    || query
                        .chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                {
                    return Err(ConfigError::new(
                        ErrorKind::Value,
                        &format!("{field}.query"),
                        "query must contain 1–512 bytes without unsupported control characters",
                    ));
                }
                Binding::Query(query)
            }
        };
        if self.kind != WidgetKind::Gauge && (self.style.is_some() || self.thresholds.is_some()) {
            return Err(ConfigError::new(
                ErrorKind::Value,
                field,
                "style and thresholds are gauge-only settings",
            ));
        }
        if self.kind != WidgetKind::Graph
            && (self.history_points.is_some()
                || self.window_secs.is_some()
                || self.max_series.is_some_and(|n| n != 1))
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                field,
                "history and multi-series settings require a graph",
            ));
        }
        if self.kind == WidgetKind::Number && (self.min.is_some() || self.max.is_some()) {
            return Err(ConfigError::new(
                ErrorKind::Value,
                field,
                "numeric widgets do not have range bounds",
            ));
        }
        if self.min.is_some_and(|v| !v.is_finite())
            || self.max.is_some_and(|v| !v.is_finite())
            || matches!((self.min,self.max),(Some(min),Some(max)) if min>=max)
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                field,
                "range bounds must be finite and strictly increasing",
            ));
        }
        let precision = self.precision.unwrap_or(3);
        if !(1..=6).contains(&precision) {
            return Err(ConfigError::new(
                ErrorKind::Value,
                &format!("{field}.precision"),
                "expected 1–6 significant digits",
            ));
        }
        let style = self.style.unwrap_or_default();
        let (min_width, min_height) = match (self.kind, style) {
            (WidgetKind::Graph, _) => (154, 94),
            (WidgetKind::Number, _) => (94, 74),
            (WidgetKind::Gauge, GaugeStyle::Bar) => (118, 94),
            (WidgetKind::Gauge, GaugeStyle::Dial) => (154, 158),
        };
        if rect.width < min_width || rect.height < min_height {
            return Err(ConfigError::new(
                ErrorKind::Layout,
                field,
                "widget is too small for its selected visualization",
            ));
        }
        let raw_thresholds = self.thresholds.unwrap_or_default().0;
        let mut thresholds = Vec::with_capacity(raw_thresholds.len());
        let mut previous = None;
        for (index, threshold) in raw_thresholds.into_iter().enumerate() {
            if !threshold.value.is_finite() || previous.is_some_and(|v| threshold.value <= v) {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &format!("{field}.thresholds[{index}].value"),
                    "thresholds must be finite and strictly increasing",
                ));
            }
            previous = Some(threshold.value);
            thresholds.push(Threshold {
                value: threshold.value,
                role: threshold.role,
                dark: threshold
                    .dark
                    .map(|v| {
                        super::theme::required_color(
                            &v,
                            &format!("{field}.thresholds[{index}].dark"),
                        )
                    })
                    .transpose()?,
                light: threshold
                    .light
                    .map(|v| {
                        super::theme::required_color(
                            &v,
                            &format!("{field}.thresholds[{index}].light"),
                        )
                    })
                    .transpose()?,
            });
        }
        let graph = self.kind == WidgetKind::Graph;
        let history_points = if graph {
            self.history_points.unwrap_or(super::DEFAULT_HISTORY_POINTS)
        } else {
            0
        };
        let max_series = self.max_series.unwrap_or(1);
        let window_secs = if graph {
            self.window_secs.unwrap_or(1800)
        } else {
            0
        };
        if graph
            && (!(1..=super::MAX_POINTS_PER_SERIES).contains(&history_points)
                || !(1..=super::MAX_SERIES_PER_WIDGET).contains(&max_series)
                || !(1..=86400).contains(&window_secs))
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                field,
                "graph history, series count, or window is out of range",
            ));
        }
        if self.aggregate != Aggregation::None
            && (source.kind() != SourceKind::Metrics || max_series != 1)
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                field,
                "mean aggregation requires a metrics source and one output series",
            ));
        }
        Ok(Widget {
            legend: self.legend,
            aggregate: self.aggregate,
            kind: self.kind,
            title: self.title,
            source: self.source,
            binding,
            placement,
            rect,
            unit: self.unit,
            scale: self.scale,
            precision,
            history_points,
            max_series,
            window_secs,
            range: Range {
                min: self.min,
                max: self.max,
            },
            style,
            thresholds,
            reservation: 0,
        })
    }
}

fn prom_name(name: &str, metric: bool) -> bool {
    let mut chars = name.bytes();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_' || (metric && first == b':'))
        && chars.all(|b| b.is_ascii_alphanumeric() || b == b'_' || (metric && b == b':'))
}
