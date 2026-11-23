//! Application wiring. Collection owns HTTP/parser staging; UI owns SPI and touch.
//! Shared history is locked only for synchronous publish or rendering, never for
//! a network await. Async mutex guards do not mask hardware interrupts.
mod calibration;
use crate::{
    board::{storage::Storage, touch::Touch},
    config::{
        self, Binding, COMPILED_CONFIG, Config, ConfigReader, Palette, SourceKind, ThemeMode,
    },
    display::{
        self, Navigation,
        board::BoardDisplay,
        controls::{self, Action},
        touch::Calibration,
    },
    metrics::{MetricSink, Model, POOL_POINTS, ParserBuffers, QueryMode, Stage, empty_points},
    network::{Network, TcpResources},
    sources::http::{ContentKind, HttpBuffers, Request},
    timing::{Ms, Span},
};
use calibration::{calibrate, wait_release};
use core::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::prelude::Point;
use static_cell::StaticCell;

static CONFIG: StaticCell<Config> = StaticCell::new();
static SCRATCH: StaticCell<[u8; 8193]> = StaticCell::new();
static ACTIVE: StaticCell<[display::Sample; POOL_POINTS]> = StaticCell::new();
static STAGING: StaticCell<[display::Sample; POOL_POINTS]> = StaticCell::new();
static HTTP: StaticCell<HttpBuffers> = StaticCell::new();
static PARSER: StaticCell<ParserBuffers> = StaticCell::new();
static TCP: StaticCell<TcpResources> = StaticCell::new();
static MODEL: Mutex<CriticalSectionRawMutex, Option<Model>> = Mutex::new(None);
static RETRY: AtomicBool = AtomicBool::new(false);
// Reserve one day before boot for initial server history, avoiding unsigned
// timestamp underflow. No wall clock is needed by the display or direct metrics.
fn now() -> u64 {
    Instant::now().as_millis().saturating_add(86_400_000)
}

/// Measures storage reads so configuration parse time can be reported separately.
struct TimedReader<'a, R> {
    inner: &'a mut R,
    read: &'a Cell<esp_hal::time::Duration>,
}
impl<R: ConfigReader> ConfigReader for TimedReader<'_, R> {
    fn read(&mut self, filename: &str, buffer: &mut [u8]) -> Result<usize, config::ReadError> {
        let start = esp_hal::time::Instant::now();
        let result = self.inner.read(filename, buffer);
        self.read.set(self.read.get() + start.elapsed());
        result
    }
}

/// Load configuration, logging storage and parse/validation time separately.
fn load_timed(
    reader: &mut impl ConfigReader,
    scratch: &mut [u8],
    attempt: u32,
) -> Result<config::LoadedConfig, config::ConfigError> {
    let read = Cell::new(esp_hal::time::Duration::ZERO);
    let span = Span::start("app: config load");
    let result = config::load(
        &mut TimedReader {
            inner: reader,
            read: &read,
        },
        scratch,
        COMPILED_CONFIG,
    );
    let total = span.done_with(format_args!("attempt {}, ok={}", attempt, result.is_ok()));
    crate::timing!(
        "app: config storage {} + parse/validate {}",
        Ms(read.get()),
        Ms(total
            .checked_sub(read.get())
            .unwrap_or(esp_hal::time::Duration::ZERO))
    );
    result
}

/// Boot selection and UI loop. A present invalid card never selects the fallback.
pub async fn run(
    spawner: Spawner,
    mut lcd: BoardDisplay,
    mut storage: Result<Storage, config::ReadError>,
    mut touch: Touch,
    wifi: esp_hal::peripherals::WIFI<'static>,
    flash: esp_hal::peripherals::FLASH<'static>,
) -> ! {
    let run_span = Span::start("app: run");
    let defaults = Palette::defaults(ThemeMode::Dark);
    let span = Span::quiet("app: loading screen draw");
    let drawn = controls::message(
        &mut lcd.lcd,
        &defaults,
        "Graphdis",
        "Loading your dashboard",
    )
    .is_ok();
    lcd.set_backlight(drawn);
    span.done_with(format_args!("backlight={}", drawn));
    let scratch = SCRATCH.init_with(|| [0; 8193]);
    let mut attempt = 0u32;
    let config = loop {
        attempt += 1;
        let result = match storage.as_mut() {
            Ok(reader) => load_timed(reader, scratch, attempt),
            Err(_) => {
                // A failed SPI initialization is an I/O failure, not an absent card.
                struct Failed;
                impl ConfigReader for Failed {
                    fn read(&mut self, _: &str, _: &mut [u8]) -> Result<usize, config::ReadError> {
                        Err(config::ReadError::Io)
                    }
                }
                load_timed(&mut Failed, scratch, attempt)
            }
        };
        match result {
            Ok(loaded) => {
                esp_println::println!(
                    "Configuration: {:?}; {} sources, {} screens, {} history points",
                    loaded.origin,
                    loaded.config.sources().len(),
                    loaded.config.screens().len(),
                    loaded.config.history_points()
                );
                break loaded.config;
            }
            Err(error) => {
                esp_println::println!("Configuration: {}", error);
                let _ = display::diagnostic(
                    &mut lcd.lcd,
                    &defaults,
                    "Configuration error",
                    "Fix TOML, then touch to retry",
                );
                crate::timing!("app: waiting for touch to retry config (user wait)");
                // Retry needs a complete contact/release cycle, avoiding a held
                // finger or noise repeatedly mounting the card.
                wait_release(&mut touch, Duration::from_secs(30)).await;
                loop {
                    Timer::after(Duration::from_millis(25)).await;
                    if touch.sample().is_some() {
                        break;
                    }
                }
                wait_release(&mut touch, Duration::from_secs(30)).await;
            }
        }
    };
    let span = Span::start("app: calibration selection");
    let mut touch_store = crate::board::calibration::CalibrationStore::new(flash);
    let mut saved = touch_store.load(storage.as_mut().ok()).ok().flatten();
    if saved.is_none()
        && let Some(coefficients) = config.touch_calibration()
    {
        crate::timing!("app: persisting calibration from TOML");
        let configured = Calibration(coefficients);
        match touch_store.save(configured, storage.as_mut().ok()) {
            Ok(location) => esp_println::println!("Configured calibration saved: {:?}", location),
            Err(error) => {
                esp_println::println!("Configured calibration persistence failed: {:?}", error)
            }
        }
        saved = Some(configured);
    }
    span.done_with(format_args!("saved={}", saved.is_some()));
    let mut calibration = if let Some(saved) = saved {
        let prompt = Span::start("app: recalibration prompt window (fixed 1.5s)");
        let _ = controls::message(
            &mut lcd.lcd,
            config.theme().active(),
            "Touch ready",
            "Tap now to redo touch setup",
        );
        let mut selected = saved;
        let mut tapped = false;
        for _ in 0..60 {
            Timer::after(Duration::from_millis(25)).await;
            if touch.sample().is_some() {
                tapped = true;
                crate::timing!("app: prompt tapped; interactive calibration (user wait)");
                if let Some(c) = calibrate(
                    &mut lcd,
                    &mut touch,
                    config.theme().active(),
                    &mut touch_store,
                    storage.as_mut().ok(),
                )
                .await
                {
                    selected = c;
                }
                break;
            }
        }
        prompt.done_with(format_args!("tapped={}", tapped));
        selected
    } else {
        crate::timing!("app: no saved calibration; interactive calibration (user wait)");
        loop {
            if let Some(c) = calibrate(
                &mut lcd,
                &mut touch,
                config.theme().active(),
                &mut touch_store,
                storage.as_mut().ok(),
            )
            .await
            {
                break c;
            }
        }
    };
    crate::timing!("app: calibration ready");
    let span = Span::start("app: model + staging init");
    let config = CONFIG.init(config);
    *MODEL.lock().await = Some(Model::new(config, ACTIVE.init_with(empty_points)));
    let stage = Stage::new(config, STAGING.init_with(empty_points));
    span.done_with(format_args!("heap free {}", esp_alloc::HEAP.free()));
    let span = Span::start("app: network start");
    let network =
        config
            .wifi()
            .and_then(|settings| match Network::start(spawner, wifi, settings) {
                Ok(n) => Some(n),
                Err(error) => {
                    esp_println::println!("Network startup: {:?}", error);
                    None
                }
            });
    span.done_with(format_args!(
        "started={} heap free {}",
        network.is_some(),
        esp_alloc::HEAP.free()
    ));
    let all = if config.reservations().len() == 16 {
        u16::MAX
    } else {
        (1u16 << config.reservations().len()) - 1
    };
    if let Some(network) = network {
        if let Some(status) = network.subscribe()
            && let Ok(task) = log_network(status)
        {
            spawner.spawn(task);
        }
        let span = Span::quiet("app: collector buffers init");
        let parser = PARSER.init_with(ParserBuffers::new);
        let http = HTTP.init_with(HttpBuffers::new);
        let tcp = TCP.init_with(TcpResources::new);
        span.done();
        match collect(config, network, stage, parser, http, tcp) {
            Ok(task) => spawner.spawn(task),
            Err(_) => {
                esp_println::println!("Collector task unavailable");
                if let Some(model) = MODEL.lock().await.as_mut() {
                    model.fail(all);
                }
            }
        }
    } else {
        esp_println::println!("Wi-Fi unavailable; add [wifi] to TOML");
        if let Some(model) = MODEL.lock().await.as_mut() {
            model.fail(all);
        }
    }
    esp_println::println!("Startup free heap: {} bytes", esp_alloc::HEAP.free());
    static RENDERER: StaticCell<display::Renderer> = StaticCell::new();
    let renderer = RENDERER.init_with(display::Renderer::new);
    run_span.done_with(format_args!("entering UI loop"));
    // Log the first few frames, the first frame showing collected data, and any
    // later frame slow enough to matter.
    let mut frames = 0u32;
    let mut data_frame_logged = false;
    let mut nav = Navigation::new(config.screens().len(), config.rotation_secs(), now());
    let mut menu = false;
    let mut captured = None;
    let mut center = false;
    let mut contact = false;
    let mut down = 0;
    let mut up = 0;
    let mut last = Point::zero();
    let mut dirty = true;
    let mut last_frame = 0;
    let mut last_revision = 0;
    loop {
        Timer::after(Duration::from_millis(20)).await;
        let time = now();
        let previous_pressed = nav.pressed();
        if let Some(raw) = touch.sample() {
            down = (down + 1).min(2);
            up = 0;
            let point = calibration.map(raw);
            last = point;
            if down >= 2 && !contact {
                contact = true;
                if menu {
                    captured = controls::hit(point);
                    dirty = true;
                } else {
                    center = (48..272).contains(&point.x) && (200..240).contains(&point.y);
                    nav.press(point);
                }
            } else if contact {
                if menu {
                    if controls::hit(point) != captured {
                        captured = None;
                        dirty = true;
                    }
                } else {
                    if !(48..272).contains(&point.x) || !(200..240).contains(&point.y) {
                        center = false;
                    }
                    nav.move_to(point);
                }
            }
        } else {
            up = (up + 1).min(3);
            down = 0;
            if up >= 3 && contact {
                contact = false;
                if menu {
                    let action = captured.take().filter(|a| controls::hit(last) == Some(*a));
                    dirty = true;
                    match action {
                        Some(Action::Back) => {
                            menu = false;
                            nav.restart_timer(time);
                        }
                        Some(Action::Retry) => {
                            RETRY.store(true, Ordering::Relaxed);
                            menu = false;
                        }
                        Some(Action::Calibrate) => {
                            renderer.invalidate();
                            // The user may have inserted a card since boot.
                            if let Ok(card) = storage.as_mut() {
                                card.reprobe();
                            }
                            if let Some(c) = calibrate(
                                &mut lcd,
                                &mut touch,
                                config.theme().active(),
                                &mut touch_store,
                                storage.as_mut().ok(),
                            )
                            .await
                            {
                                calibration = c;
                            }
                            menu = false;
                        }
                        Some(Action::Reload) => {
                            renderer.invalidate();
                            if let Ok(card) = storage.as_mut() {
                                card.reprobe();
                            }
                            let _ = controls::message(
                                &mut lcd.lcd,
                                config.theme().active(),
                                "Reloading",
                                "Reading graphdis.toml",
                            );
                            let replacement = match storage.as_mut() {
                                Ok(reader) => config::load(reader, scratch, COMPILED_CONFIG),
                                Err(_) => Err(config::ConfigError::storage_failure()),
                            };
                            match replacement {
                                Ok(_) => {
                                    esp_println::println!("Valid reload; restarting firmware");
                                    esp_hal::system::software_reset();
                                }
                                Err(error) => {
                                    esp_println::println!("Reload rejected: {}", error);
                                    let _ = display::diagnostic(
                                        &mut lcd.lcd,
                                        config.theme().active(),
                                        "Reload rejected",
                                        "Current dashboard remains active",
                                    );
                                    Timer::after(Duration::from_secs(3)).await;
                                }
                            }
                        }
                        None => {}
                    }
                    if !menu {
                        nav.restart_timer(now());
                    }
                } else if center {
                    menu = true;
                    center = false;
                    dirty = true;
                    nav.release(Point::new(-1, -1), time);
                } else {
                    dirty |= nav.release(last, time);
                }
            }
        }
        let time = now();
        dirty |= nav.pressed() != previous_pressed;
        if !menu && !contact && down == 0 {
            dirty |= nav.tick(time);
        }
        let mut model = MODEL.lock().await;
        if let Some(model) = model.as_mut() {
            model.now_ms = time;
            dirty |= model.revision != last_revision || time.saturating_sub(last_frame) >= 1000;
            if dirty {
                let frame = Span::quiet("ui: frame");
                let yields = Cell::new(0u32);
                let yield_time = Cell::new(esp_hal::time::Duration::ZERO);
                let result = if menu {
                    renderer.menu(
                        &mut lcd.lcd,
                        config.theme().active(),
                        captured,
                        if network.is_some_and(|n| n.is_ready()) {
                            "Wi-Fi online"
                        } else {
                            "Wi-Fi offline"
                        },
                    )
                } else {
                    let yields = &yields;
                    let yield_time = &yield_time;
                    renderer
                        .draw_cooperative(&mut lcd.lcd, config, &nav, model, time, || async move {
                            let start = esp_hal::time::Instant::now();
                            Timer::after(Duration::from_millis(1)).await;
                            yields.set(yields.get() + 1);
                            yield_time.set(yield_time.get() + start.elapsed());
                        })
                        .await
                };
                let tiles = result.as_ref().ok().map(|stats| stats.tiles_written);
                lcd.set_backlight(result.is_ok());
                frames = frames.saturating_add(1);
                let elapsed = frame.elapsed();
                let first_data = !data_frame_logged && model.revision != 0 && !menu;
                if frames <= 5 || first_data || elapsed.as_millis() >= 100 {
                    crate::timing!(
                        "ui: frame {} {}{} tiles={:?} yields={} in {} rev={}",
                        frames,
                        Ms(elapsed),
                        if first_data { " FIRST DATA" } else { "" },
                        tiles,
                        yields.get(),
                        Ms(yield_time.get()),
                        model.revision
                    );
                }
                data_frame_logged |= first_data;
                if result.is_err() {
                    renderer.invalidate();
                    esp_println::println!("Display frame failed");
                }
                last_revision = model.revision;
                last_frame = time;
                dirty = false;
            }
        }
    }
}

#[embassy_executor::task]
async fn collect(
    config: &'static Config,
    network: Network,
    mut stage: Stage,
    parser: &'static mut ParserBuffers,
    http: &'static mut HttpBuffers,
    tcp: &'static TcpResources,
) {
    crate::timing!("collect: task started");
    let client = network.http_client(tcp);
    let mut first_commit = false;
    let mut waiting = Some(Span::quiet("collect: waiting for network"));
    let mut due = [0u64; 24];
    let mut delays = [1u32; 24];
    let mut clocks = [None::<(u64, u64)>; 8];
    let mut clock_due = [0u64; 8];
    let mut offline = false;
    loop {
        if RETRY.swap(false, Ordering::Relaxed) {
            due.fill(0);
            delays.fill(1);
        }
        if !network.is_ready() {
            if !offline {
                if let Some(m) = MODEL.lock().await.as_mut() {
                    m.fail(u16::MAX);
                }
                offline = true;
            }
            Timer::after(Duration::from_millis(250)).await;
            continue;
        }
        if let Some(span) = waiting.take() {
            span.done();
        }
        if offline {
            crate::timing!("collect: network back; refetching all");
            due.fill(0);
            offline = false;
        }
        for (source_index, (name, source)) in config.sources().iter().enumerate() {
            let source_mask = Stage::mask_for_source(config, name);
            if source_mask == 0 {
                continue;
            }
            // time() supplies the Prometheus server's evaluation clock even if
            // the selected metric has no current sample. No Internet NTP is needed.
            if source.kind() == SourceKind::Prometheus && now() >= clock_due[source_index] {
                let time = now();
                let group = source_mask.trailing_zeros() as usize;
                let mut sink = MetricSink::new(
                    config,
                    &mut stage,
                    parser,
                    1 << group,
                    QueryMode::Instant { group },
                    time,
                );
                let span = Span::quiet("collect: fetch");
                let result = client
                    .fetch(
                        source,
                        Request::Instant {
                            query: "time()",
                            time_ms: None,
                        },
                        http,
                        &mut sink,
                    )
                    .await;
                span.done_with(format_args!(
                    "source {} time() ok={} bytes={:?}",
                    name,
                    result.is_ok(),
                    result.as_ref().ok().map(|s| s.body_bytes)
                ));
                if result.is_ok()
                    && let Some(unix) = stage.server_ms()
                {
                    clocks[source_index] = Some((unix, time));
                    clock_due[source_index] = now() + 3_600_000;
                } else {
                    clock_due[source_index] = now() + 60_000;
                }
            }
            for group in 0..config.reservations().len() {
                if source_mask & (1 << group) == 0 {
                    continue;
                }
                let direct = source.kind() == SourceKind::Metrics;
                let key = if direct { source_index } else { 8 + group };
                let time = now();
                if time < due[key] {
                    continue;
                }
                let mask = if direct { source_mask } else { 1 << group };
                let reservation = config.reservations()[group];
                let widget = &config.screens()[reservation.screen].widgets[reservation.widget];
                let (request, mode) = if direct {
                    (Request::Metrics, QueryMode::Metrics)
                } else {
                    let Binding::Query(query) = &widget.binding else {
                        continue;
                    };
                    if reservation.points_per_series > 1
                        && let Some((unix, mono)) = clocks[source_index]
                    {
                        let end = unix.saturating_add(time.saturating_sub(mono));
                        let window = u64::from(widget.window_secs) * 1000;
                        let step = window
                            .div_ceil(reservation.points_per_series.saturating_sub(1) as u64)
                            .max(1)
                            .min(u64::from(u32::MAX)) as u32;
                        (
                            Request::Range {
                                query,
                                start_ms: end.saturating_sub(window),
                                end_ms: end,
                                step_ms: step,
                            },
                            QueryMode::Range {
                                group,
                                end_unix_ms: end,
                            },
                        )
                    } else {
                        (
                            Request::Instant {
                                query,
                                time_ms: None,
                            },
                            QueryMode::Instant { group },
                        )
                    }
                };

                let kind = match request {
                    Request::Metrics => "metrics",
                    Request::Instant { .. } => "instant",
                    Request::Range { .. } => "range",
                };
                let mut sink = MetricSink::new(config, &mut stage, parser, mask, mode, time);
                let span = Span::quiet("collect: fetch");
                let result = client.fetch(source, request, http, &mut sink).await;
                span.done_with(format_args!(
                    "source {} {} group {} ok={} bytes={:?}",
                    name,
                    kind,
                    group,
                    result.is_ok(),
                    result.as_ref().ok().map(|s| s.body_bytes)
                ));
                let success = result.as_ref().is_ok_and(|s| {
                    if direct {
                        matches!(
                            s.content_type,
                            ContentKind::Text | ContentKind::OpenMetrics | ContentKind::Unknown
                        )
                    } else {
                        matches!(s.content_type, ContentKind::Json | ContentKind::Unknown)
                    }
                });
                let finished = now();
                if success {
                    let span = Span::quiet("collect: commit");
                    if let Some(model) = MODEL.lock().await.as_mut() {
                        model.commit(&stage, finished);
                    }
                    span.done();
                    if !first_commit {
                        first_commit = true;
                        crate::timing!("collect: FIRST DATA committed");
                    }
                    delays[key] = 1;
                    // Keep the polling cadence independent of request duration.
                    due[key] = time + u64::from(source.poll_interval_secs()) * 1000;
                } else {
                    if let Some(model) = MODEL.lock().await.as_mut() {
                        model.fail(mask);
                    }
                    match result {
                        Err(error) => esp_println::println!("Source {} failed: {:?}", name, error),
                        Ok(summary) => esp_println::println!(
                            "Source {} rejected content type: {:?}",
                            name,
                            summary.content_type
                        ),
                    }
                    due[key] = finished + u64::from(delays[key]) * 1000;
                    delays[key] = (delays[key] * 2).min(30);
                }
                if direct {
                    break;
                }
                Timer::after(Duration::from_millis(1)).await;
            }
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[embassy_executor::task]
async fn log_network(mut status: crate::network::StatusReceiver) {
    loop {
        let status = status.changed().await;
        crate::timing!("Network: {:?}", status);
    }
}
