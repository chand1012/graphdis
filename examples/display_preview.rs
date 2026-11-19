//! Host-only rendering of the actual embedded UI into portable PPM images.
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use graphdis::{
    config::Config,
    display::{self, Data, Navigation, Sample, Series, State, WidgetData},
};
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
};
struct Canvas(Vec<Rgb565>);
impl Dimensions for Canvas {
    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(320, 240))
    }
}
impl DrawTarget for Canvas {
    type Color = Rgb565;
    type Error = core::convert::Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error> {
        for Pixel(p, c) in pixels {
            if (0..320).contains(&p.x) && (0..240).contains(&p.y) {
                self.0[p.y as usize * 320 + p.x as usize] = c;
            }
        }
        Ok(())
    }
}
struct Demo {
    history: [Sample; 120],
    cpu: [Sample; 1],
    memory: [Sample; 1],
}
struct Snapshot<'a> {
    graphs: &'a [Series<'a>],
    cpu: &'a [Series<'a>],
    memory: &'a [Series<'a>],
}
impl Data for Snapshot<'_> {
    fn with_widget<R>(&self, reservation: usize, draw: impl FnOnce(WidgetData<'_>) -> R) -> R {
        draw(WidgetData {
            state: State::Ready,
            observed_range: None,
            series: match reservation {
                0 => self.graphs,
                1 => self.cpu,
                _ => self.memory,
            },
        })
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/display-preview"));
    fs::create_dir_all(&out)?;
    let mut demo = Demo {
        history: [Sample {
            timestamp_ms: 0,
            value: 0.0,
        }; 120],
        cpu: [Sample {
            timestamp_ms: 1_800_000,
            value: 76.4,
        }],
        memory: [Sample {
            timestamp_ms: 1_800_000,
            value: 3_221_225_472.0,
        }],
    };
    for (i, s) in demo.history.iter_mut().enumerate() {
        s.timestamp_ms = i as u64 * 15_000;
        s.value = 120.0 + (i as f64 / 9.0).sin() * 35.0 + i as f64 * 0.4;
    }
    let mut second = demo.history;
    for s in &mut second {
        s.value *= 0.6;
    }
    let graphs = [
        Series {
            identity: "",
            color_slot: 0,
            label: "GET /api",
            samples: &demo.history,
        },
        Series {
            identity: "",
            color_slot: 1,
            label: "GET /health",
            samples: &second,
        },
    ];
    let cpu = [Series {
        identity: "",
        color_slot: 0,
        label: "cpu",
        samples: &demo.cpu,
    }];
    let memory = [Series {
        identity: "",
        color_slot: 0,
        label: "memory",
        samples: &demo.memory,
    }];
    let data = Snapshot {
        graphs: &graphs,
        cpu: &cpu,
        memory: &memory,
    };
    for mode in ["dark", "light"] {
        let raw = include_str!("../config/example.toml")
            .replace("mode = \"dark\"", &format!("mode = \"{mode}\""));
        let cfg = Config::parse(&raw).map_err(|e| format!("{e}"))?;
        let mut nav = Navigation::new(cfg.screens().len(), Some(1), 0);
        for index in 0..cfg.screens().len() {
            let mut canvas = Canvas(vec![Rgb565::BLACK; 320 * 240]);
            display::render(&mut canvas, &cfg, &nav, &data, 1_800_000).unwrap();
            let path = out.join(format!("{mode}-{index}.ppm"));
            let mut file = File::create(&path)?;
            write!(file, "P6\n320 240\n255\n")?;
            for p in canvas.0 {
                file.write_all(&[
                    (u16::from(p.r()) * 255 / 31) as u8,
                    (u16::from(p.g()) * 255 / 63) as u8,
                    (u16::from(p.b()) * 255 / 31) as u8,
                ])?;
            }
            println!("{}", path.display());
            nav.tick((index as u64 + 1) * 1000);
        }
    }
    Ok(())
}
