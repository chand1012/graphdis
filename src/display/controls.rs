//! Small system controls share the dashboard's fonts, colors, and release rules.
use crate::config::{Color, Palette};
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        iso_8859_1::{FONT_6X12, FONT_10X20},
    },
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Retry,
    Reload,
    Calibrate,
    Back,
}
fn rect(x: i32, y: i32, w: u32, h: u32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}
fn rgb(c: Color) -> Rgb565 {
    RawU16::new(c.rgb565()).into()
}
fn label<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    s: &str,
    r: Rectangle,
    c: Color,
    large: bool,
) -> Result<(), D::Error> {
    let font = if large { &FONT_10X20 } else { &FONT_6X12 };
    let width = s.chars().count() as i32 * font.character_size.width as i32;
    Text::with_baseline(
        s,
        Point::new(
            r.top_left.x + (r.size.width as i32 - width) / 2,
            r.top_left.y,
        ),
        MonoTextStyle::new(font, rgb(c)),
        Baseline::Top,
    )
    .draw(d)
    .map(|_| ())
}
fn area(a: Action) -> Rectangle {
    match a {
        Action::Retry => rect(8, 56, 148, 44),
        Action::Reload => rect(164, 56, 148, 44),
        Action::Calibrate => rect(8, 108, 304, 44),
        Action::Back => rect(0, 200, 320, 40),
    }
}
pub fn hit(p: Point) -> Option<Action> {
    [
        Action::Retry,
        Action::Reload,
        Action::Calibrate,
        Action::Back,
    ]
    .into_iter()
    .find(|a| area(*a).contains(p))
}
pub fn menu<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    p: &Palette,
    pressed: Option<Action>,
    status: &str,
) -> Result<(), D::Error> {
    d.clear(rgb(p.background))?;
    label(d, status, rect(8, 18, 304, 20), p.text, true)?;
    for (a, s) in [
        (Action::Retry, "Retry sources"),
        (Action::Reload, "Reload TOML"),
        (Action::Calibrate, "Calibrate touch"),
        (Action::Back, "Back"),
    ] {
        let r = area(a);
        d.fill_solid(
            &r,
            rgb(if pressed == Some(a) {
                p.raised_surface
            } else {
                p.surface
            }),
        )?;
        r.into_styled(PrimitiveStyle::with_stroke(
            rgb(if pressed == Some(a) {
                p.accent
            } else {
                p.border
            }),
            1,
        ))
        .draw(d)?;
        label(
            d,
            s,
            rect(r.top_left.x, r.top_left.y + 15, r.size.width, 12),
            p.text,
            false,
        )?;
    }
    label(
        d,
        "Reload validates, then restarts",
        rect(8, 169, 304, 12),
        p.secondary_text,
        false,
    )
}
pub fn message<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    p: &Palette,
    title: &str,
    detail: &str,
) -> Result<(), D::Error> {
    d.clear(rgb(p.background))?;
    label(d, title, rect(8, 70, 304, 20), p.accent, true)?;
    label(d, detail, rect(8, 110, 304, 12), p.text, false)
}
pub fn cross<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    p: &Palette,
    point: Point,
) -> Result<(), D::Error> {
    message(d, p, "Touch calibration", "Touch each cross, then release")?;
    let style = PrimitiveStyle::with_stroke(rgb(p.accent), 2);
    Line::new(point - Point::new(8, 0), point + Point::new(8, 0))
        .into_styled(style)
        .draw(d)?;
    Line::new(point - Point::new(0, 8), point + Point::new(0, 8))
        .into_styled(style)
        .draw(d)
}

pub fn calibration_intro<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    p: &Palette,
) -> Result<(), D::Error> {
    d.clear(rgb(p.background))?;
    label(d, "Touch setup", rect(8, 28, 304, 20), p.accent, true)?;
    for (y, s) in [
        (72, "1. Touch the numbered target"),
        (94, "2. Hold until it turns green"),
        (116, "3. Lift to continue"),
        (158, "Setup saves automatically"),
        (202, "Tap anywhere to begin"),
    ] {
        label(d, s, rect(8, y, 304, 12), p.text, false)?;
    }
    Ok(())
}
pub fn calibration_target<D: DrawTarget<Color = Rgb565>>(
    d: &mut D,
    p: &Palette,
    point: Point,
    step: usize,
    held: bool,
) -> Result<(), D::Error> {
    use core::fmt::Write;
    d.clear(rgb(p.background))?;
    let mut title = heapless::String::<32>::new();
    let _ = write!(title, "Target {} of 4", step + 1);
    label(d, &title, rect(8, 62, 304, 20), p.text, true)?;
    label(
        d,
        if held {
            "Green: lift your finger"
        } else {
            "Touch the cross and hold still"
        },
        rect(8, 94, 304, 12),
        p.text,
        false,
    )?;
    let style = PrimitiveStyle::with_stroke(rgb(if held { p.success } else { p.accent }), 2);
    Line::new(point - Point::new(10, 0), point + Point::new(10, 0))
        .into_styled(style)
        .draw(d)?;
    Line::new(point - Point::new(0, 10), point + Point::new(0, 10))
        .into_styled(style)
        .draw(d)
}
