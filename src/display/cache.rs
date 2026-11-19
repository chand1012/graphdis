//! Tile comparisons suppress unchanged SPI writes without a full framebuffer.
use super::{
    Data, Navigation, State, WidgetData,
    controls::{self, Action},
};
use crate::config::{Config, Palette, WidgetKind};
use core::convert::Infallible;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
const TW: u32 = 32;
const TH: u32 = 16;
const COLS: usize = 10;
const COUNT: usize = 150;
fn hash(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
fn word(h: u64, v: u64) -> u64 {
    hash(h, &v.to_le_bytes())
}
const SEED: u64 = 0xcbf29ce484222325;
fn signature(view: WidgetData<'_>, graph: bool, now: u64) -> u64 {
    let mut h = word(
        SEED,
        match view.state {
            State::Loading => 0,
            State::Ready => 1,
            State::NoData => 2,
            State::Error => 3,
            State::Stale { age_secs } => 4 + u64::from(age_secs),
        },
    );
    if let Some((a, b)) = view.observed_range {
        h = word(word(h, a.to_bits()), b.to_bits());
    }
    h = word(h, view.series.len() as u64);
    for s in view.series {
        h = hash(word(h, s.color_slot as u64), s.label.as_bytes());
        h = hash(h, s.identity.as_bytes());
        if graph {
            for p in s.samples {
                h = word(word(h, p.timestamp_ms), p.value.to_bits());
            }
        } else if let Some(p) = s.samples.last() {
            h = word(h, p.value.to_bits());
        }
    }
    if graph {
        h = word(h, now / 1000);
    }
    h
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    pub tiles_written: usize,
}
/// About 2.5 KiB: one 32×16 RGB565 tile plus hashes, never a full frame.
/// Call invalidate after direct drawing or configuration changes.
pub struct Renderer {
    pixels: [Rgb565; 512],
    hashes: [u64; COUNT],
    valid: [bool; COUNT],
    screen: Option<usize>,
    widgets: [u64; 16],
    footer: u64,
    menu_signature: Option<u64>,
}
impl Renderer {
    pub const fn new() -> Self {
        Self {
            pixels: [Rgb565::BLACK; 512],
            hashes: [0; COUNT],
            valid: [false; COUNT],
            screen: None,
            widgets: [0; 16],
            footer: 0,
            menu_signature: None,
        }
    }
    pub fn invalidate(&mut self) {
        self.valid.fill(false);
        self.screen = None;
        self.menu_signature = None;
    }
    pub fn draw<D: DrawTarget<Color = Rgb565>>(
        &mut self,
        target: &mut D,
        config: &Config,
        nav: &Navigation,
        data: &impl Data,
        now: u64,
    ) -> Result<DrawStats, D::Error> {
        let (regions, signatures, footer, full) = self.prepare(config, nav, data, now);
        let stats = self.paint(
            target,
            |r| {
                full || regions.iter().any(|dirty| {
                    let s = r.intersection(dirty).size;
                    s.width > 0 && s.height > 0
                })
            },
            |tile| super::render(tile, config, nav, data, now),
        )?;
        self.finish(nav, signatures, footer);
        Ok(stats)
    }
    /// Yield between tiles so graph rasterization cannot starve the network executor.
    pub async fn draw_cooperative<D, F>(
        &mut self,
        target: &mut D,
        config: &Config,
        nav: &Navigation,
        data: &impl Data,
        now: u64,
        mut yield_tile: impl FnMut() -> F,
    ) -> Result<DrawStats, D::Error>
    where
        D: DrawTarget<Color = Rgb565>,
        F: core::future::Future<Output = ()>,
    {
        let (regions, signatures, footer, full) = self.prepare(config, nav, data, now);
        let mut stats = DrawStats::default();
        for index in 0..COUNT {
            let r = tile_rect(index);
            if !self.valid[index]
                || full
                || regions.iter().any(|dirty| {
                    let s = r.intersection(dirty).size;
                    s.width > 0 && s.height > 0
                })
            {
                stats.tiles_written += self.paint_tile(target, index, &mut |tile| {
                    super::render(tile, config, nav, data, now)
                })?;
                yield_tile().await;
            }
        }
        self.finish(nav, signatures, footer);
        Ok(stats)
    }
    fn prepare(
        &self,
        config: &Config,
        nav: &Navigation,
        data: &impl Data,
        now: u64,
    ) -> (heapless::Vec<Rectangle, 17>, [u64; 16], u64, bool) {
        let full = self.screen != Some(nav.index()) || self.menu_signature.is_some();
        let mut regions = heapless::Vec::<Rectangle, 17>::new();
        let mut signatures = self.widgets;
        if let Some(screen) = config.screens().get(nav.index()) {
            for (i, w) in screen.widgets.iter().enumerate() {
                let sig = data.with_widget(w.reservation, |view| {
                    signature(view, w.kind == WidgetKind::Graph, now)
                });
                if full || sig != self.widgets[i] {
                    let r = w.rect;
                    let _ = regions.push(Rectangle::new(
                        Point::new(i32::from(r.x), i32::from(r.y)),
                        Size::new(u32::from(r.width), u32::from(r.height)),
                    ));
                }
                signatures[i] = sig;
            }
        }
        let footer = word(
            word(SEED, nav.index() as u64),
            match nav.pressed() {
                None => 0,
                Some(super::Button::Previous) => 1,
                Some(super::Button::Next) => 2,
            },
        );
        if full || footer != self.footer {
            let _ = regions.push(Rectangle::new(Point::new(0, 200), Size::new(320, 40)));
        }
        (regions, signatures, footer, full)
    }
    fn finish(&mut self, nav: &Navigation, signatures: [u64; 16], footer: u64) {
        self.screen = Some(nav.index());
        self.widgets = signatures;
        self.footer = footer;
        self.menu_signature = None;
    }
    pub fn menu<D: DrawTarget<Color = Rgb565>>(
        &mut self,
        target: &mut D,
        p: &Palette,
        pressed: Option<Action>,
        status: &str,
    ) -> Result<DrawStats, D::Error> {
        let sig = word(
            hash(SEED, status.as_bytes()),
            match pressed {
                None => 0,
                Some(Action::Retry) => 1,
                Some(Action::Reload) => 2,
                Some(Action::Calibrate) => 3,
                Some(Action::Back) => 4,
            },
        );
        if self.menu_signature == Some(sig) {
            return Ok(DrawStats::default());
        }
        let stats = self.paint(
            target,
            |_| true,
            |tile| controls::menu(tile, p, pressed, status),
        )?;
        self.menu_signature = Some(sig);
        self.screen = None;
        Ok(stats)
    }
    fn paint<D: DrawTarget<Color = Rgb565>>(
        &mut self,
        target: &mut D,
        mut selected: impl FnMut(Rectangle) -> bool,
        mut draw: impl FnMut(&mut Tile<'_>) -> Result<(), Infallible>,
    ) -> Result<DrawStats, D::Error> {
        let mut stats = DrawStats::default();
        for index in 0..COUNT {
            let r = tile_rect(index);
            if self.valid[index] && !selected(r) {
                continue;
            }
            stats.tiles_written += self.paint_tile(target, index, &mut draw)?;
        }
        Ok(stats)
    }
    fn paint_tile<D: DrawTarget<Color = Rgb565>>(
        &mut self,
        target: &mut D,
        index: usize,
        draw: &mut impl FnMut(&mut Tile<'_>) -> Result<(), Infallible>,
    ) -> Result<usize, D::Error> {
        let r = tile_rect(index);
        self.pixels.fill(Rgb565::BLACK);
        match draw(&mut Tile {
            rect: r,
            pixels: &mut self.pixels,
        }) {
            Ok(()) => {}
            Err(never) => match never {},
        }
        let mut h = SEED;
        for p in &self.pixels {
            h = hash(h, &p.into_storage().to_le_bytes());
        }
        if !self.valid[index] || self.hashes[index] != h {
            target.fill_contiguous(&r, self.pixels.iter().copied())?;
            self.hashes[index] = h;
            self.valid[index] = true;
            return Ok(1);
        }
        Ok(0)
    }
}
impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}
fn tile_rect(index: usize) -> Rectangle {
    Rectangle::new(
        Point::new(
            (index % COLS) as i32 * TW as i32,
            (index / COLS) as i32 * TH as i32,
        ),
        Size::new(TW, TH),
    )
}
struct Tile<'a> {
    rect: Rectangle,
    pixels: &'a mut [Rgb565; 512],
}
impl Dimensions for Tile<'_> {
    fn bounding_box(&self) -> Rectangle {
        self.rect
    }
}
impl DrawTarget for Tile<'_> {
    type Color = Rgb565;
    type Error = Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Infallible> {
        for Pixel(p, c) in pixels {
            if self.rect.contains(p) {
                let q = p - self.rect.top_left;
                self.pixels[q.y as usize * TW as usize + q.x as usize] = c;
            }
        }
        Ok(())
    }
    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Infallible> {
        let r = area.intersection(&self.rect);
        for p in r.points() {
            let q = p - self.rect.top_left;
            self.pixels[q.y as usize * TW as usize + q.x as usize] = color;
        }
        Ok(())
    }
}
