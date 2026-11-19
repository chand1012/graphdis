extern crate std;
use super::*;
use crate::config::{Config, Scale};
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use std::{vec, vec::Vec};

struct Canvas {
    pixels: Vec<Rgb565>,
}
impl Canvas {
    fn new() -> Self {
        Self {
            pixels: vec![Rgb565::BLACK; 320 * 240],
        }
    }
}
impl Dimensions for Canvas {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(320, 240))
    }
}
impl DrawTarget for Canvas {
    type Color = Rgb565;
    type Error = ();
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(&mut self, pixels: I) -> Result<(), ()> {
        for Pixel(p, c) in pixels {
            assert!((0..320).contains(&p.x) && (0..240).contains(&p.y));
            self.pixels[p.y as usize * 320 + p.x as usize] = c;
        }
        Ok(())
    }
}
struct Values<'a>(&'a [Series<'a>], State);
impl Data for Values<'_> {
    fn with_widget<R>(&self, _: usize, draw: impl FnOnce(WidgetData<'_>) -> R) -> R {
        draw(WidgetData {
            state: self.1,
            observed_range: None,
            series: self.0,
        })
    }
}

#[test]
fn navigation_wraps_cancels_and_restarts_timer() {
    let mut nav = Navigation::new(3, Some(10), 0);
    nav.press(Point::new(20, 220));
    assert!(nav.release(Point::new(20, 220), 8000));
    assert_eq!(nav.index(), 2);
    assert!(!nav.tick(10000));
    assert!(nav.tick(18000));
    assert_eq!(nav.index(), 0);
    nav.press(Point::new(290, 220));
    nav.move_to(Point::new(250, 220));
    nav.move_to(Point::new(290, 220));
    assert!(!nav.release(Point::new(290, 220), 19000));
    assert!(!nav.release(Point::new(290, 220), 19000));
    assert!(nav.tick(28000));
    assert_eq!(nav.index(), 1);
}
#[test]
fn single_screen_disables_touch_and_rotation() {
    let mut n = Navigation::new(1, Some(1), 0);
    n.press(Point::new(290, 220));
    assert_eq!(n.pressed(), None);
    assert!(!n.release(Point::new(290, 220), 1));
    assert!(!n.tick(10000));
}
#[test]
fn numeric_and_label_formatting() {
    assert_eq!(format::number(1536.0, Scale::Binary, 3).unwrap(), "1.5Ki");
    assert_eq!(format::number(12345.0, Scale::Decimal, 3).unwrap(), "12.3k");
    assert_eq!(format::number(0.000025, Scale::Decimal, 3).unwrap(), "25µ");
    assert_eq!(format::number(12.345, Scale::None, 3).unwrap(), "12.3");
    assert_eq!(format::number(12345.0, Scale::None, 3).unwrap(), "12300");
    assert!(format::number(f64::NAN, Scale::Decimal, 3).is_none());
    assert_eq!(format::label("CPU μ ° µ 🙂", 12), "CPU ? ° µ ?");
    assert_eq!(format::label("abcdéfg", 5), "ab...");
    assert_eq!(format::label("abcd", 2), "..");
}
#[test]
fn every_widget_theme_and_state_stays_inside_canvas() {
    let raw = include_str!("../../config/example.toml");
    let readings = [
        Sample {
            timestamp_ms: 0,
            value: 10.0,
        },
        Sample {
            timestamp_ms: 500,
            value: f64::NAN,
        },
        Sample {
            timestamp_ms: 1000,
            value: 25.0,
        },
    ];
    let series = [
        Series {
            identity: "",
            color_slot: 0,
            label: "server-a",
            samples: &readings,
        },
        Series {
            identity: "",
            color_slot: 1,
            label: "server-b",
            samples: &readings,
        },
        Series {
            identity: "",
            color_slot: 2,
            label: "server-c",
            samples: &readings,
        },
    ];
    for theme in ["dark", "light"] {
        let cfg =
            Config::parse(&raw.replace("mode = \"dark\"", &std::format!("mode = \"{theme}\"")))
                .unwrap();
        let mut nav = Navigation::new(cfg.screens().len(), Some(1), 0);
        let mut canvas = Canvas::new();
        for state in [
            State::Loading,
            State::Ready,
            State::NoData,
            State::Error,
            State::Stale { age_secs: 90 },
        ] {
            render(&mut canvas, &cfg, &nav, &Values(&series, state), 1000).unwrap();
            assert!(nav.tick(1000));
            render(&mut canvas, &cfg, &nav, &Values(&series, state), 1000).unwrap();
            nav.press(Point::new(20, 220));
            nav.release(Point::new(20, 220), 0);
        }
    }
}
#[test]
fn display_errors_propagate() {
    struct Broken;
    impl Dimensions for Broken {
        fn bounding_box(&self) -> Rectangle {
            Rectangle::new(Point::zero(), Size::new(320, 240))
        }
    }
    impl DrawTarget for Broken {
        type Color = Rgb565;
        type Error = u8;
        fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(&mut self, _: I) -> Result<(), u8> {
            Err(7)
        }
    }
    let cfg = Config::parse(crate::config::COMPILED_CONFIG).unwrap();
    assert_eq!(
        render(&mut Broken, &cfg, &Navigation::new(1, None, 0), &Loading, 0),
        Err(7)
    );
}
#[test]
fn graph_draws_series_and_keeps_reserved_navigation() {
    let cfg = Config::parse(include_str!("../../config/example.toml")).unwrap();
    let readings = [
        Sample {
            timestamp_ms: 0,
            value: 10.0,
        },
        Sample {
            timestamp_ms: 1000,
            value: 25.0,
        },
    ];
    let series = [Series {
        identity: "",
        color_slot: 0,
        label: "server-a",
        samples: &readings,
    }];
    let mut canvas = Canvas::new();
    render(
        &mut canvas,
        &cfg,
        &Navigation::new(2, None, 0),
        &Values(&series, State::Ready),
        1000,
    )
    .unwrap();
    let color = Rgb565::from(embedded_graphics::pixelcolor::raw::RawU16::new(
        cfg.theme().active().chart_color(0).rgb565(),
    ));
    assert!(canvas.pixels[..200 * 320].contains(&color));
    assert!(!canvas.pixels[200 * 320..].contains(&color));
}

#[test]
fn gauge_extrema_ignore_invalid_values_and_reset_on_reload() {
    let mut tracker = GaugeRange::default();
    assert_eq!(tracker.bounds(), None);
    for value in [f64::NAN, f64::INFINITY, 42.0, -10.0, 100.0, 12.0] {
        tracker.observe(value);
    }
    assert_eq!(tracker.bounds(), Some((-10.0, 100.0)));
    tracker.reset();
    assert_eq!(tracker.bounds(), None);
    for value in [f64::MAX, -f64::MAX, f64::MIN_POSITIVE, 0.0, -0.0] {
        let _ = format::number(value, Scale::Decimal, 3);
        let _ = format::number(value, Scale::Binary, 3);
        let _ = format::number(value, Scale::None, 3);
    }
}

struct Recorded {
    canvas: Canvas,
    writes: Vec<Rectangle>,
}
impl Recorded {
    fn new() -> Self {
        Self {
            canvas: Canvas::new(),
            writes: Vec::new(),
        }
    }
}
impl Dimensions for Recorded {
    fn bounding_box(&self) -> Rectangle {
        self.canvas.bounding_box()
    }
}
impl DrawTarget for Recorded {
    type Color = Rgb565;
    type Error = ();
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(&mut self, pixels: I) -> Result<(), ()> {
        self.canvas.draw_iter(pixels)
    }
    fn fill_contiguous<I: IntoIterator<Item = Rgb565>>(
        &mut self,
        area: &Rectangle,
        colors: I,
    ) -> Result<(), ()> {
        self.writes.push(*area);
        self.canvas
            .draw_iter(area.points().zip(colors).map(|(p, c)| Pixel(p, c)))
    }
}
#[test]
fn tile_renderer_matches_full_frame_and_skips_unchanged_writes() {
    let cfg = Config::parse(include_str!("../../config/example.toml")).unwrap();
    let samples = [Sample {
        timestamp_ms: 1000,
        value: 42.0,
    }];
    let series = [Series {
        identity: "",
        color_slot: 0,
        label: "Metric",
        samples: &samples,
    }];
    let data = Values(&series, State::Ready);
    let mut nav = Navigation::new(2, None, 0);
    let mut renderer = Renderer::new();
    let mut panel = Recorded::new();
    let mut reference = Canvas::new();
    let first = renderer.draw(&mut panel, &cfg, &nav, &data, 1000).unwrap();
    assert_eq!(first.tiles_written, 150);
    render(&mut reference, &cfg, &nav, &data, 1000).unwrap();
    assert_eq!(panel.canvas.pixels, reference.pixels);
    panel.writes.clear();
    assert_eq!(
        renderer
            .draw(&mut panel, &cfg, &nav, &data, 1000)
            .unwrap()
            .tiles_written,
        0
    );
    assert!(panel.writes.is_empty());
    nav.press(Point::new(20, 220));
    let pressed = renderer.draw(&mut panel, &cfg, &nav, &data, 1000).unwrap();
    assert!(pressed.tiles_written > 0 && pressed.tiles_written < 10);
    assert!(
        panel
            .writes
            .iter()
            .all(|r| r.top_left.y >= 192 && r.top_left.x < 64)
    );
    render(&mut reference, &cfg, &nav, &data, 1000).unwrap();
    assert_eq!(panel.canvas.pixels, reference.pixels);
    nav.release(Point::new(20, 220), 1000);
    panel.writes.clear();
    renderer.draw(&mut panel, &cfg, &nav, &data, 1000).unwrap();
    render(&mut reference, &cfg, &nav, &data, 1000).unwrap();
    assert_eq!(panel.canvas.pixels, reference.pixels);
}
#[test]
fn changed_number_erases_old_digits_without_redrawing_footer_or_whole_widget() {
    let cfg=Config::parse("[sources.a]\ntype='metrics'\nurl='http://127.0.0.1/metrics'\n[[screens]]\ntitle='Number'\n[[screens.widgets]]\ntype='number'\ntitle='Value'\nsource='a'\nmetric='value'\nscale='none'\n").unwrap();
    let nav = Navigation::new(1, None, 0);
    let mut renderer = Renderer::new();
    let mut panel = Recorded::new();
    for number in [1000.0, 7.0, 42.001, 42.002] {
        let samples = [Sample {
            timestamp_ms: 1000,
            value: number,
        }];
        let series = [Series {
            identity: "",
            color_slot: 0,
            label: "",
            samples: &samples,
        }];
        let data = Values(&series, State::Ready);
        panel.writes.clear();
        let stats = renderer.draw(&mut panel, &cfg, &nav, &data, 1000).unwrap();
        let mut reference = Canvas::new();
        render(&mut reference, &cfg, &nav, &data, 1000).unwrap();
        assert_eq!(panel.canvas.pixels, reference.pixels);
        if number != 1000.0 {
            assert!(stats.tiles_written < 12);
            assert!(panel.writes.iter().all(|r| r.top_left.y < 192));
        }
        if number == 42.002 {
            assert_eq!(stats.tiles_written, 0);
        }
    }
}
#[test]
fn saved_calibration_rejects_corruption_and_recovers_previous_slot() {
    use super::touch::{Calibration, newest};
    let calibration = Calibration::from_points(
        [(200, 200), (3800, 200), (200, 3800)],
        [Point::new(20, 20), Point::new(300, 20), Point::new(20, 180)],
    )
    .unwrap();
    let a = calibration.encode(1);
    let b = calibration.encode(2);
    assert_eq!(
        newest([Calibration::decode(&a), Calibration::decode(&b)]),
        Some(1)
    );
    for i in 0..64 {
        let mut damaged = b;
        damaged[i] ^= 1;
        assert!(Calibration::decode(&damaged).is_none());
        assert_eq!(
            newest([Calibration::decode(&a), Calibration::decode(&damaged)]),
            Some(0)
        );
    }
    assert!(Calibration::decode(&[0xff; 64]).is_none());
    assert_eq!(
        newest([
            Calibration::decode(&calibration.encode(u32::MAX)),
            Calibration::decode(&calibration.encode(0))
        ]),
        Some(1)
    );
}

#[test]
fn holding_navigation_defers_rotation_and_edge_jitter_does_not_cancel_a_tap() {
    let mut nav = Navigation::new(3, Some(1), 0);
    nav.press(Point::new(290, 220));
    assert!(!nav.tick(1500));
    assert_eq!(nav.pressed(), Some(Button::Next));
    nav.move_to(Point::new(270, 220)); // two pixels across the edge while held
    nav.move_to(Point::new(290, 220));
    assert!(nav.release(Point::new(290, 220), 1600));
    assert_eq!(nav.index(), 1);
    assert!(!nav.tick(2599));
    assert!(nav.tick(2600));
    nav.press(Point::new(290, 220));
    nav.move_to(Point::new(260, 220));
    nav.move_to(Point::new(290, 220));
    assert!(!nav.release(Point::new(290, 220), 3000));
}

#[derive(Default)]
struct CalibrationCard {
    fail_read: Option<usize>,
    files: [Option<[u8; 64]>; 2],
    readonly: bool,
    cut_write: bool,
    changed: Vec<usize>,
}
impl super::calibration_files::CalibrationFiles for CalibrationCard {
    type Error = ();
    fn read(&mut self, slot: usize) -> Result<Option<[u8; 64]>, ()> {
        if self.fail_read == Some(slot) {
            return Err(());
        }
        Ok(self.files[slot])
    }
    fn write(&mut self, slot: usize, record: &[u8; 64]) -> Result<(), ()> {
        if self.readonly {
            return Err(());
        }
        self.changed.push(slot);
        if self.cut_write {
            let mut partial = [0xff; 64];
            partial[..24].copy_from_slice(&record[..24]);
            self.files[slot] = Some(partial);
            return Err(());
        }
        self.files[slot] = Some(*record);
        Ok(())
    }
}
#[test]
fn sd_calibration_survives_reopen_and_rotates_files() {
    use super::{
        calibration_files::{load, save},
        touch::{Calibration, newest},
    };
    let calibration = Calibration::default();
    let mut card = CalibrationCard::default();
    assert!(load(&mut card).unwrap().iter().all(Option::is_none));
    save(&mut card, calibration, 1).unwrap();
    let stored = card.files;
    // Reconstruct the reader, as on a new power cycle, from persistent bytes.
    let mut reopened = CalibrationCard {
        files: stored,
        ..CalibrationCard::default()
    };
    let r = load(&mut reopened).unwrap();
    assert_eq!(newest(r), Some(0));
    assert_eq!(r[0].unwrap().1.map((200, 200)), calibration.map((200, 200)));
    save(&mut reopened, calibration, 2).unwrap();
    assert_eq!(reopened.changed, [1]);
    let r = load(&mut reopened).unwrap();
    assert_eq!(newest(r), Some(1));
    assert_eq!(r[1].unwrap().0, 2);
}
#[test]
fn interrupted_or_readonly_sd_saves_preserve_previous_valid_record() {
    use super::{
        calibration_files::{load, save},
        touch::{Calibration, newest},
    };
    let calibration = Calibration::default();
    let mut card = CalibrationCard::default();
    save(&mut card, calibration, 10).unwrap();
    let original = card.files[0];
    card.cut_write = true;
    assert!(save(&mut card, calibration, 11).is_err());
    assert_eq!(card.files[0], original);
    let records = load(&mut card).unwrap();
    assert_eq!(newest(records), Some(0));
    assert_eq!(records[0].unwrap().0, 10);
    assert!(records[1].is_none());
    card.cut_write = false;
    card.readonly = true;
    assert!(save(&mut card, calibration, 11).is_err());
    assert_eq!(card.files[0], original);
    // A newer flash backup wins over the stale SD copy after a failed SD save.
    assert_eq!(newest([Some((11, calibration)), records[0]]), Some(0));
    // Equal generations prefer SD, and absent flash still permits SD recovery.
    assert_eq!(newest([records[0], records[0]]), Some(1));
    assert_eq!(newest([None, records[0]]), Some(1));
}

#[test]
fn sd_calibration_recovers_when_only_one_file_is_readable() {
    use super::{
        calibration_files::{load, save},
        touch::{Calibration, newest},
    };
    let mut card = CalibrationCard::default();
    let c = Calibration::default();
    save(&mut card, c, 1).unwrap();
    save(&mut card, c, 2).unwrap();
    card.fail_read = Some(0);
    let r = load(&mut card).unwrap();
    assert_eq!(newest(r), Some(1));
    assert_eq!(r[1].unwrap().0, 2);
    card.fail_read = Some(1);
    let r = load(&mut card).unwrap();
    assert_eq!(newest(r), Some(0));
    card.files[0] = None;
    assert!(load(&mut card).is_err());
}

#[test]
fn legend_templates_use_original_labels_and_fit_the_available_cells() {
    use super::format::{label, legend};
    let identity = "3:foo3:cpu1:08:instance11:host:9100,a";
    assert_eq!(
        legend(Some("Core {{cpu}} ({{instance}})"), "default", identity),
        "Core 0 (host:9100,a)"
    );
    assert_eq!(
        legend(Some("Friendly name"), "default", identity),
        "Friendly name"
    );
    assert_eq!(legend(Some("{{__name__}}"), "default", identity), "foo");
    assert_eq!(legend(Some("{{missing}}"), "default", identity), "default");
    assert_eq!(legend(None, "default", identity), "default");
    let long = legend(
        Some("{{instance}}"),
        "default",
        "3:foo8:instance60:abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefgh",
    );
    assert_eq!(label(&long, 20), "abcdefghijklmnopq...");
}

#[test]
fn cooperative_renderer_yields_between_tiles_and_matches_synchronous_output() {
    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll},
    };
    let cfg = Config::parse(include_str!("../../config/example.toml")).unwrap();
    let nav = Navigation::new(cfg.screens().len(), None, 0);
    let samples = [Sample {
        timestamp_ms: 1000,
        value: 42.0,
    }];
    let series = [Series {
        color_slot: 0,
        label: "Test",
        identity: "",
        samples: &samples,
    }];
    let data = Values(&series, State::Ready);
    let mut expected = Canvas::new();
    let mut actual = Canvas::new();
    let sync_stats = Renderer::new()
        .draw(&mut expected, &cfg, &nav, &data, 1000)
        .unwrap();
    let mut renderer = Renderer::new();
    let mut cx = Context::from_waker(std::task::Waker::noop());
    {
        let mut future =
            pin!(
                renderer.draw_cooperative(&mut actual, &cfg, &nav, &data, 1000, || {
                    let mut yielded = false;
                    core::future::poll_fn(move |cx| {
                        if yielded {
                            Poll::Ready(())
                        } else {
                            yielded = true;
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                    })
                })
            );
        let mut yields = 0;
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Pending => yields += 1,
                Poll::Ready(result) => {
                    assert_eq!(result.unwrap(), sync_stats);
                    break;
                }
            }
        }
        assert_eq!(yields, 150);
    }
    assert_eq!(actual.pixels, expected.pixels);
}
