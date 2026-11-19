//! Render a saved exposition response through the actual collector and UI.
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use graphdis::{
    config::Config,
    display::{Navigation, Renderer},
    metrics::{MetricSink, Model, ParserBuffers, QueryMode, Stage, empty_points},
    sources::http::BodySink,
};
use std::{
    boxed::Box,
    fs::{self, File},
    future::Future,
    io::Write,
    path::PathBuf,
    pin::pin,
    task::{Context, Poll},
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
fn ready<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("parser unexpectedly waited"),
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let config_path = args
        .next()
        .ok_or("usage: render_metrics CONFIG METRICS [OUTPUT_DIR]")?;
    let metrics_path = args.next().ok_or("missing metrics file")?;
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/metrics-preview"));
    let config = Config::parse(&fs::read_to_string(config_path)?)?;
    let metrics = fs::read(metrics_path)?;
    let mut model = Model::new(&config, Box::leak(Box::new(empty_points())));
    let mut stage = Stage::new(&config, Box::leak(Box::new(empty_points())));
    let mut parser = ParserBuffers::new();
    let source = config
        .sources()
        .iter()
        .find(|(_, s)| s.kind() == graphdis::config::SourceKind::Metrics)
        .ok_or("no exposition source")?
        .0;
    let mask = Stage::mask_for_source(&config, source);
    let mut sink = MetricSink::new(
        &config,
        &mut stage,
        &mut parser,
        mask,
        QueryMode::Metrics,
        86_400_000,
    );
    sink.begin();
    for chunk in metrics.chunks(73) {
        ready(sink.write(chunk)).map_err(|_| "metrics parsing failed")?;
    }
    sink.finish().map_err(|_| "metrics finalization failed")?;
    model.commit(&stage, 86_400_000);
    fs::create_dir_all(&output)?;
    let mut nav = Navigation::new(config.screens().len(), Some(1), 0);
    let mut renderer = Renderer::new();
    let mut canvas = Canvas(vec![Rgb565::BLACK; 320 * 240]);
    for i in 0..config.screens().len() {
        renderer
            .draw(&mut canvas, &config, &nav, &model, 86_400_000)
            .unwrap();
        let path = output.join(format!("screen-{i}.ppm"));
        let mut file = File::create(&path)?;
        write!(file, "P6\n320 240\n255\n")?;
        for p in &canvas.0 {
            file.write_all(&[
                (u16::from(p.r()) * 255 / 31) as u8,
                (u16::from(p.g()) * 255 / 63) as u8,
                (u16::from(p.b()) * 255 / 31) as u8,
            ])?;
        }
        println!("{}", path.display());
        nav.tick((i as u64 + 1) * 1000);
    }
    Ok(())
}
