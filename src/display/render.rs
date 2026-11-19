use super::{
    Button, Data, Navigation, Sample, State, WidgetData,
    format::{label, legend, number},
};
use crate::config::{Color, Config, GaugeStyle, Palette, Range, Widget, WidgetKind};
use core::fmt::Write;
use embedded_graphics::{
    mono_font::{
        MonoFont, MonoTextStyle,
        iso_8859_1::{FONT_6X10, FONT_6X12, FONT_7X13_BOLD, FONT_10X20},
    },
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use heapless::String;

fn rgb(c: Color) -> Rgb565 {
    RawU16::new(c.rgb565()).into()
}
fn rect(x: i32, y: i32, w: u32, h: u32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}
fn fill<D: DrawTarget<Color = Rgb565>>(d: &mut D, r: Rectangle, c: Color) -> Result<(), D::Error> {
    d.fill_solid(&r, rgb(c))
}
fn line<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    a: Point,
    b: Point,
    c: Color,
    width: u32,
) -> Result<(), D::Error> {
    Line::new(a, b)
        .into_styled(PrimitiveStyle::with_stroke(rgb(c), width))
        .draw(d)
}
fn text<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    s: &str,
    p: Point,
    font: &MonoFont<'_>,
    c: Color,
) -> Result<(), D::Error> {
    Text::with_baseline(s, p, MonoTextStyle::new(font, rgb(c)), Baseline::Top)
        .draw(d)
        .map(|_| ())
}
fn centered<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    s: &str,
    r: Rectangle,
    font: &MonoFont<'_>,
    c: Color,
) -> Result<(), D::Error> {
    let s = label(s, (r.size.width / font.character_size.width) as usize);
    let w = s.chars().count() as i32 * font.character_size.width as i32;
    text(
        d,
        &s,
        Point::new(r.top_left.x + (r.size.width as i32 - w) / 2, r.top_left.y),
        font,
        c,
    )
}

/// Render the selected screen. All drawing is clipped to the canvas and each
/// widget; borrowed readings must use the same epoch as now_ms.
pub fn render<D: DrawTarget<Color = Rgb565>>(
    target: &mut D,
    config: &Config,
    nav: &Navigation,
    data: &impl Data,
    now_ms: u64,
) -> Result<(), D::Error> {
    let mut d = target.clipped(&rect(0, 0, 320, 240));
    let p = config.theme().active();
    fill(&mut d, rect(0, 0, 320, 200), p.background)?;
    let index = nav.index().min(config.screens().len().saturating_sub(1));
    if let Some(screen) = config.screens().get(index) {
        if screen.widgets.is_empty() {
            centered(
                &mut d,
                "Configure Graphdis",
                rect(12, 56, 296, 20),
                &FONT_10X20,
                p.text,
            )?;
            centered(
                &mut d,
                "Add widgets to graphdis.toml",
                rect(12, 88, 296, 12),
                &FONT_6X12,
                p.secondary_text,
            )?;
        }
        for widget in &screen.widgets {
            let r = widget.rect;
            let r = rect(
                i32::from(r.x),
                i32::from(r.y),
                u32::from(r.width),
                u32::from(r.height),
            );
            let visible = r.intersection(&d.bounding_box()).size;
            if visible.width == 0 || visible.height == 0 {
                continue;
            }
            let mut wd = d.clipped(&r);
            fill(&mut wd, r, p.surface)?;
            r.into_styled(PrimitiveStyle::with_stroke(rgb(p.border), 1))
                .draw(&mut wd)?;
            let inner = rect(
                r.top_left.x + 7,
                r.top_left.y + 7,
                r.size.width - 14,
                r.size.height - 14,
            );
            let mut wd = wd.clipped(&inner);
            data.with_widget(widget.reservation, |view| {
                draw_widget(&mut wd, widget, view, inner, config, p, now_ms)
            })?;
        }
        let footer_visible = rect(0, 200, 320, 40).intersection(&d.bounding_box()).size;
        if footer_visible.width > 0 && footer_visible.height > 0 {
            fill(&mut d, rect(0, 200, 320, 40), p.surface)?;
            line(
                &mut d,
                Point::new(0, 200),
                Point::new(319, 200),
                p.border,
                1,
            )?;
            for (button, x, symbol) in [(Button::Previous, 0, "<"), (Button::Next, 272, ">")] {
                if nav.pressed() == Some(button) {
                    fill(&mut d, rect(x, 201, 48, 39), p.raised_surface)?;
                    rect(x, 200, 48, 40)
                        .into_styled(PrimitiveStyle::with_stroke(rgb(p.accent), 1))
                        .draw(&mut d)?;
                }
                centered(
                    &mut d,
                    symbol,
                    rect(x, 213, 48, 12),
                    &FONT_6X12,
                    if config.screens().len() > 1 {
                        p.text
                    } else {
                        p.secondary_text
                    },
                )?;
            }
            let title = label(&screen.title, 27);
            let mut caption = String::<128>::new();
            let _ = write!(
                caption,
                "{}  {}/{}",
                title,
                index + 1,
                config.screens().len()
            );
            centered(&mut d, &caption, rect(48, 213, 224, 12), &FONT_6X12, p.text)?;
        }
    }
    Ok(())
}

/// Safe diagnostic text only: never pass raw config, URLs, or source responses.
pub fn diagnostic<D: DrawTarget<Color = Rgb565>>(
    target: &mut D,
    p: &Palette,
    title: &str,
    message: &str,
) -> Result<(), D::Error> {
    let mut d = target.clipped(&rect(0, 0, 320, 240));
    fill(&mut d, rect(0, 0, 320, 240), p.background)?;
    centered(&mut d, title, rect(8, 72, 304, 20), &FONT_10X20, p.error)?;
    centered(&mut d, message, rect(8, 108, 304, 12), &FONT_6X12, p.text)
}

fn draw_widget<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    w: &Widget,
    data: WidgetData<'_>,
    r: Rectangle,
    config: &Config,
    p: &Palette,
    now_ms: u64,
) -> Result<(), D::Error> {
    let title_width = if matches!(data.state, State::Stale { .. }) {
        r.size.width.saturating_sub(72)
    } else {
        r.size.width
    };
    text(
        d,
        &label(&w.title, (title_width / 7) as usize),
        r.top_left,
        &FONT_7X13_BOLD,
        p.text,
    )?;
    if let State::Stale { age_secs } = data.state {
        let mut age = String::<24>::new();
        if age_secs >= 3600 {
            let _ = write!(age, "STALE {}h", age_secs / 3600);
        } else if age_secs >= 60 {
            let _ = write!(age, "STALE {}m", age_secs / 60);
        } else {
            let _ = write!(age, "STALE {}s", age_secs);
        }
        let age = label(&age, 12);
        text(
            d,
            &age,
            Point::new(
                r.top_left.x + r.size.width as i32 - age.chars().count() as i32 * 6,
                r.top_left.y + 1,
            ),
            &FONT_6X10,
            p.warning,
        )?;
    }
    let body = rect(
        r.top_left.x,
        r.top_left.y + 18,
        r.size.width,
        r.size.height.saturating_sub(18),
    );
    let message = match data.state {
        State::Loading => Some(("Loading...", p.secondary_text)),
        State::NoData => Some(("No data", p.secondary_text)),
        State::Error => Some(("Error", p.error)),
        _ => None,
    };
    if let Some((msg, c)) = message {
        return centered(
            d,
            msg,
            rect(
                body.top_left.x,
                body.top_left.y + body.size.height.saturating_sub(12) as i32 / 2,
                body.size.width,
                12,
            ),
            &FONT_6X12,
            c,
        );
    }
    if data.series.is_empty() {
        return centered(d, "No data", body, &FONT_6X12, p.secondary_text);
    }
    if w.kind == WidgetKind::Graph {
        return graph(d, w, data, body, p, now_ms);
    }
    if data.series.len() > 1 {
        return centered(d, "Ambiguous", body, &FONT_6X12, p.error);
    }
    let Some(sample) = data.series[0].samples.last() else {
        return centered(d, "No data", body, &FONT_6X12, p.secondary_text);
    };
    if !sample.value.is_finite() {
        return centered(d, "Invalid", body, &FONT_6X12, p.error);
    }
    if w.kind == WidgetKind::Number {
        return value(
            d,
            w,
            sample.value,
            rect(
                body.top_left.x,
                body.top_left.y + body.size.height.saturating_sub(20) as i32 / 2,
                body.size.width,
                20,
            ),
            p,
        );
    }
    let observed = data
        .observed_range
        .filter(|(a, b)| a.is_finite() && b.is_finite() && a <= b)
        .unwrap_or((sample.value, sample.value));
    let range = range(w.range, [observed.0, observed.1, sample.value].into_iter());
    let ratio = position(sample.value, range);
    let color = w
        .gauge_color(sample.value, config.theme().mode, p)
        .unwrap_or(p.accent);
    let range_y = if w.style == GaugeStyle::Dial {
        // A semicircle formed by short radial strokes, no framebuffer or floats
        // retained between frames. 61 segments remain readable at minimum size.
        let radius = (body.size.width / 2)
            .min(body.size.height.saturating_sub(47))
            .max(1) as f64;
        let cx = body.top_left.x + body.size.width as i32 / 2;
        let cy = body.top_left.y + radius as i32;
        for i in 0..=60 {
            let angle = core::f64::consts::PI * (1.0 - f64::from(i) / 60.0);
            let (sin, cos) = (libm::sin(angle), libm::cos(angle));
            let a = Point::new(
                cx + (cos * (radius - 5.0)) as i32,
                cy - (sin * (radius - 5.0)) as i32,
            );
            let b = Point::new(cx + (cos * radius) as i32, cy - (sin * radius) as i32);
            line(
                d,
                a,
                b,
                if f64::from(i) / 60.0 <= ratio {
                    color
                } else {
                    p.border
                },
                2,
            )?;
        }
        value(
            d,
            w,
            sample.value,
            rect(body.top_left.x, cy + 7, body.size.width, 20),
            p,
        )?;
        cy + 31
    } else {
        value(d, w, sample.value, body, p)?;
        let bar = rect(body.top_left.x, body.top_left.y + 26, body.size.width, 8);
        fill(d, bar, p.border)?;
        if ratio > 0.0 {
            fill(
                d,
                rect(
                    bar.top_left.x,
                    bar.top_left.y,
                    (ratio * f64::from(bar.size.width)) as u32,
                    8,
                ),
                color,
            )?;
        }
        body.top_left.y + 38
    };
    let label_width = body.size.width / 2 - 4;
    for (v, right) in [(range.0, false), (range.1, true)] {
        let n = number(v, w.scale, 2);
        let n = n
            .as_ref()
            .filter(|n| n.chars().count() as u32 * 6 <= label_width)
            .map_or("...", |n| n.as_str());
        let x = body.top_left.x
            + if right {
                body.size.width as i32 - n.chars().count() as i32 * 6
            } else {
                0
            };
        text(d, n, Point::new(x, range_y), &FONT_6X10, p.secondary_text)?;
    }
    let warning = if sample.value < range.0 {
        "Below range"
    } else if sample.value > range.1 {
        "Above range"
    } else {
        ""
    };
    if !warning.is_empty() {
        centered(
            d,
            warning,
            rect(
                body.top_left.x,
                body.top_left.y + body.size.height as i32 - 10,
                body.size.width,
                10,
            ),
            &FONT_6X10,
            p.warning,
        )?;
    }
    Ok(())
}

fn value<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    w: &Widget,
    v: f64,
    r: Rectangle,
    p: &Palette,
) -> Result<(), D::Error> {
    let unit = label(&w.unit, 16);
    let unit_width = unit.chars().count() as u32 * 6;
    for digits in (1..=w.precision).rev() {
        if let Some(n) = number(v, w.scale, digits) {
            let nw = n.chars().count() as u32 * 10;
            let total = nw + if unit.is_empty() { 0 } else { 4 + unit_width };
            if total <= r.size.width {
                let x = r.top_left.x + (r.size.width - total) as i32 / 2;
                text(d, &n, Point::new(x, r.top_left.y), &FONT_10X20, p.text)?;
                if !unit.is_empty() {
                    text(
                        d,
                        &unit,
                        Point::new(x + nw as i32 + 4, r.top_left.y + 7),
                        &FONT_6X12,
                        p.secondary_text,
                    )?;
                }
                return Ok(());
            }
        }
    }
    centered(d, "Overflow", r, &FONT_6X12, p.error)
}

/// Use scaled arithmetic to avoid overflow for finite values near f64::MAX.
fn position(v: f64, (min, max): (f64, f64)) -> f64 {
    let scale = min.abs().max(max.abs()).max(v.abs()).max(1.0);
    if min >= max {
        return if v <= min { 0.0 } else { 1.0 };
    }
    ((v / scale - min / scale) / (max / scale - min / scale)).clamp(0.0, 1.0)
}
fn range(bounds: Range, values: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for v in values.filter(|v| v.is_finite()) {
        low = low.min(v);
        high = high.max(v);
    }
    if !low.is_finite() {
        low = 0.0;
        high = 1.0;
    }
    let pad = if low == high {
        low.abs().max(1.0) * 0.05
    } else {
        (high * 0.05 - low * 0.05).max(f64::MIN_POSITIVE)
    };
    let mut min = bounds.min.unwrap_or((low - pad).max(-f64::MAX));
    let mut max = bounds.max.unwrap_or((high + pad).min(f64::MAX));
    if min >= max {
        if bounds.max.is_none() {
            max = (min + min.abs().max(1.0) * 0.1).min(f64::MAX);
        }
        if bounds.min.is_none() {
            min = (max - max.abs().max(1.0) * 0.1).max(-f64::MAX);
        }
    }
    (min, max)
}

fn samples<'a>(w: &Widget, s: &super::Series<'a>, now: u64) -> impl Iterator<Item = &'a Sample> {
    s.samples
        .iter()
        .skip(
            s.samples
                .len()
                .saturating_sub(usize::from(w.history_points)),
        )
        .filter(move |s| {
            s.timestamp_ms <= now && now - s.timestamp_ms <= u64::from(w.window_secs) * 1000
        })
}
fn graph<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    w: &Widget,
    data: WidgetData<'_>,
    r: Rectangle,
    p: &Palette,
    now: u64,
) -> Result<(), D::Error> {
    let series = &data.series[..data.series.len().min(usize::from(w.max_series))];
    if !series.iter().any(|s| samples(w, s, now).next().is_some()) {
        return centered(d, "No data", r, &FONT_6X12, p.secondary_text);
    }
    if !series
        .iter()
        .any(|s| samples(w, s, now).any(|v| v.value.is_finite()))
    {
        return centered(d, "Invalid", r, &FONT_6X12, p.error);
    }
    let legend_rows = series.len().min(if r.size.height >= 110 { 4 } else { 2 });
    let legend_height = legend_rows as u32 * 11;
    let plot = rect(
        r.top_left.x + 36,
        r.top_left.y,
        r.size.width.saturating_sub(37),
        r.size.height.saturating_sub(legend_height + 14),
    );
    if plot.size.width < 2 || plot.size.height < 2 {
        return Ok(());
    }
    let bounds = range(
        w.range,
        series
            .iter()
            .flat_map(|s| samples(w, s, now).map(|s| s.value)),
    );
    let vertical_intervals = if plot.size.height >= 42 { 2 } else { 1 };
    for tick in 0..=vertical_intervals {
        let y = plot.top_left.y + ((plot.size.height - 1) * tick / vertical_intervals) as i32;
        line(
            d,
            Point::new(plot.top_left.x, y),
            Point::new(plot.top_left.x + plot.size.width as i32 - 1, y),
            p.border,
            1,
        )?;

        let v = bounds.1 * (1.0 - f64::from(tick) / f64::from(vertical_intervals))
            + bounds.0 * (f64::from(tick) / f64::from(vertical_intervals));
        let n = number(v, w.scale, 2).unwrap_or_else(|| String::try_from("...").unwrap());
        // Axis precision may not truncate values; mark those that do not fit.
        text(
            d,
            if n.chars().count() <= 5 { &n } else { "..." },
            Point::new(
                r.top_left.x,
                y.min(plot.top_left.y + plot.size.height as i32 - 10),
            ),
            &FONT_6X10,
            p.secondary_text,
        )?;
    }
    for tick in 0..=2 {
        let x = plot.top_left.x + ((plot.size.width - 1) * tick / 2) as i32;
        line(
            d,
            Point::new(x, plot.top_left.y),
            Point::new(x, plot.top_left.y + plot.size.height as i32 - 1),
            p.border,
            1,
        )?;
    }
    let mut duration = String::<24>::new();
    if w.window_secs >= 60 {
        let _ = write!(duration, "-{}m", w.window_secs / 60);
    } else {
        let _ = write!(duration, "-{}s", w.window_secs);
    }
    let axis_y = plot.top_left.y + plot.size.height as i32 + 2;
    text(
        d,
        &duration,
        Point::new(plot.top_left.x, axis_y),
        &FONT_6X10,
        p.secondary_text,
    )?;
    text(
        d,
        "now",
        Point::new(plot.top_left.x + plot.size.width as i32 - 18, axis_y),
        &FONT_6X10,
        p.secondary_text,
    )?;
    let mut half = String::<24>::new();
    if w.window_secs >= 120 {
        let _ = write!(half, "-{}m", w.window_secs / 120);
    } else {
        let _ = write!(half, "-{}s", w.window_secs / 2);
    }
    if plot.size.width >= 100 {
        centered(
            d,
            &half,
            rect(plot.top_left.x, axis_y, plot.size.width, 10),
            &FONT_6X10,
            p.secondary_text,
        )?;
    }
    {
        let mut pd = d.clipped(&plot);
        for s in series {
            let mut previous = None;
            for sample in samples(w, s, now) {
                if !sample.value.is_finite() {
                    previous = None;
                    continue;
                }
                let age = now - sample.timestamp_ms;
                let x = plot.top_left.x
                    + ((1.0 - age as f64 / (f64::from(w.window_secs) * 1000.0))
                        * f64::from(plot.size.width - 1)) as i32;
                let y = plot.top_left.y
                    + ((1.0 - position(sample.value, bounds)) * f64::from(plot.size.height - 1))
                        as i32;
                let point = Point::new(x, y);
                let thickness = if plot.size.width >= 180 && plot.size.height >= 80 {
                    2
                } else {
                    1
                };
                if let Some(last) = previous {
                    line(&mut pd, last, point, p.chart_color(s.color_slot), thickness)?;
                } else {
                    Pixel(point, rgb(p.chart_color(s.color_slot))).draw(&mut pd)?;
                }
                previous = Some(point);
            }
        }
    }
    for (slot, s) in series.iter().take(legend_rows).enumerate() {
        let y = r.top_left.y + r.size.height as i32 - legend_height as i32 + slot as i32 * 11;
        fill(
            d,
            rect(r.top_left.x, y + 3, 5, 5),
            p.chart_color(s.color_slot),
        )?;
        let mut latest = String::<64>::new();
        if let Some(sample) = samples(w, s, now).last() {
            if sample.value.is_finite() {
                if let Some(v) = number(sample.value, w.scale, w.precision) {
                    let _ = write!(latest, "{}{}", v, label(&w.unit, 16));
                }
            } else {
                let _ = latest.push_str("Invalid");
            }
        }
        let caption = legend(w.legend.as_deref(), s.label, s.identity);
        let cells = (r.size.width.saturating_sub(9) / 6) as usize;
        // If series overflow, reserve the final legend row for a count.
        if slot + 1 == legend_rows && series.len() > legend_rows {
            let mut more = String::<32>::new();
            let _ = write!(more, "+{} series", series.len() - slot);
            text(
                d,
                &more,
                Point::new(r.top_left.x + 9, y),
                &FONT_6X10,
                p.secondary_text,
            )?;
        } else {
            let latest_cells = latest.chars().count();
            if cells >= latest_cells + 9 && !latest.is_empty() {
                text(
                    d,
                    &label(&caption, cells - latest_cells - 1),
                    Point::new(r.top_left.x + 9, y),
                    &FONT_6X10,
                    p.text,
                )?;
                text(
                    d,
                    &latest,
                    Point::new(
                        r.top_left.x + r.size.width as i32 - latest_cells as i32 * 6,
                        y,
                    ),
                    &FONT_6X10,
                    p.text,
                )?;
            } else {
                text(
                    d,
                    &label(&caption, cells),
                    Point::new(r.top_left.x + 9, y),
                    &FONT_6X10,
                    p.text,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_handle_empty_constant_extreme_and_partial_bounds() {
        assert_eq!(range(Range::default(), core::iter::empty()), (-0.05, 1.05));
        let (a, b) = range(Range::default(), [42.0, 42.0, f64::NAN].into_iter());
        assert!(a < 42.0 && b > 42.0);
        let (a, b) = range(
            Range {
                min: Some(100.0),
                max: None,
            },
            [1.0].into_iter(),
        );
        assert_eq!(a, 100.0);
        assert!(b > a);
        let bounds = range(Range::default(), [-f64::MAX, f64::MAX].into_iter());
        assert_eq!(position(0.0, bounds), 0.5);
        assert_eq!(position(f64::MAX, bounds), 1.0);
    }
}
