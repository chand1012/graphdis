use super::*;
use alloc::{
    format,
    string::{String, ToString},
    vec,
};

const EMPTY: &str = "[[screens]]\ntitle='Setup'\n";
const SOURCE: &str = "[sources.local]\ntype='metrics'\nurl='http://exporter:9100/metrics'\n";

fn single(extra: &str) -> String {
    format!(
        "{SOURCE}\n[[screens]]\ntitle='Main'\n[[screens.widgets]]\ntype='graph'\ntitle='Requests'\nsource='local'\nmetric='requests_total'\n{extra}\n"
    )
}

#[test]
fn defaults_and_full_example_are_valid() {
    let default = Config::parse(include_str!("../../config/default.toml")).unwrap();
    assert!(default.wifi().is_none());
    assert_eq!(default.screens().len(), 1);
    assert_eq!(default.theme().mode, ThemeMode::Dark);
    assert_eq!(default.theme().dark.background.rgb888(), 0x111217);
    assert_eq!(default.theme().light.surface.rgb888(), 0xffffff);
    assert_eq!(default.theme().dark.chart_colors().len(), 6);
    assert_eq!(default.history_points(), 0);
    let full = Config::parse(include_str!("../../config/example.toml")).unwrap();
    assert_eq!(full.sources().len(), 2);
    assert_eq!(full.screens().len(), 2);
    assert_eq!(full.rotation_secs(), Some(30));
    assert_eq!(full.history_points(), 360);
    assert_eq!(full.tracked_series(), 6);
    assert_eq!(
        full.screens()[1].widgets[0].reservation,
        full.screens()[1].widgets[2].reservation
    );
}

#[test]
fn every_level_rejects_unknown_keys_and_old_wifi_only_documents() {
    for document in [
        "",
        "[wifi]\nssid='home'\npassword='12345678'",
        "[[screens]]\ntitle='Main'\nunknown=true",
    ] {
        assert!(Config::parse(document).is_err());
    }
    for extra in [
        "[wiif]\nssid='typo'",
        "[theme]\nunknown=true",
        "[theme.light]\ntextt='#FFFFFF'",
        "[display]\nrotaton_secs=30",
    ] {
        assert!(Config::parse(&format!("{EMPTY}\n{extra}")).is_err());
    }
    assert!(Config::parse(&single("histroy_points=120")).is_err());
    assert!(Config::parse(&format!("version=2\n{EMPTY}")).is_err());
}

#[test]
fn validates_wifi_and_redacts_credentials() {
    let parse = |ssid: &str, password: &str| {
        Config::parse(&format!(
            "[wifi]\nssid={ssid:?}\npassword={password:?}\n{EMPTY}"
        ))
    };
    assert!(parse(&"é".repeat(16), "12345678").is_ok());
    assert_eq!(
        parse(&"é".repeat(17), "12345678").unwrap_err().field,
        "wifi.ssid"
    );
    assert!(parse("", "12345678").is_err());
    assert!(parse("home", "short").is_err());
    assert!(parse("home", &"x".repeat(63)).is_ok());
    assert!(parse("home", &"a".repeat(64)).is_ok());
    assert!(parse("home", &"x".repeat(64)).is_err());
    assert!(parse("home", &"a".repeat(65)).is_err());
    assert!(parse("home", "éééééééé").is_err());
    assert!(parse("home", "").is_err());
    assert!(
        Config::parse(&format!(
            "[wifi]\nssid='guest'\nauthentication='open'\n{EMPTY}"
        ))
        .is_ok()
    );
    assert!(
        Config::parse(&format!(
            "[wifi]\nssid='guest'\nauthentication='open'\npassword='12345678'\n{EMPTY}"
        ))
        .is_err()
    );
    assert!(
        Config::parse(&format!(
            "[wifi]\nssid=\"home\\u0000\"\npassword='12345678'\n{EMPTY}"
        ))
        .is_err()
    );
    let valid = parse("private-network", "private-password").unwrap();
    let debug = format!("{valid:?} {:?}", valid.wifi());
    assert!(!debug.contains("private-network") && !debug.contains("private-password"));
}

#[test]
fn parser_errors_report_positions_without_echoing_input() {
    let error = Config::parse("[wifi]\nssid=123\npassword='sensitive-value'\n").unwrap_err();
    assert_eq!(error.kind, ErrorKind::Syntax);
    assert!(error.position.is_some());
    assert!(!format!("{error:?} {error}").contains("sensitive-value"));
    assert_eq!(
        Config::parse(&"#".repeat(MAX_CONFIG_BYTES + 1))
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
}

#[test]
fn validates_both_theme_modes_and_resolves_partial_overrides() {
    let config = Config::parse(&format!(
        "{EMPTY}\n[theme.dark]\naccent='#aBcDeF'\nchart_colors=['#FF0000','#00FF00']"
    ))
    .unwrap();
    assert_eq!(config.theme().dark.accent.rgb888(), 0xabcdef);
    assert_eq!(config.theme().light.accent.rgb888(), 0x1f62e0);
    assert_eq!(config.theme().dark.chart_colors().len(), 2);
    assert_eq!(config.theme().dark.chart_color(2).rgb888(), 0xff0000);
    for value in [
        "#FFF",
        "#FFFFFFFF",
        "#12345G",
        " #123456",
        "red",
        "123456",
        "#123456\n",
    ] {
        assert!(Color::parse(value).is_none());
    }
    for extra in [
        "[theme.light]\naccent='#FFF'",
        "[theme.light]\nchart_colors=[]",
        "[theme]\nmode='auto'",
    ] {
        assert!(Config::parse(&format!("{EMPTY}\n{extra}")).is_err());
    }
    let colors = vec!["'#FFFFFF'"; 13].join(",");
    assert!(Config::parse(&format!("{EMPTY}\n[theme.dark]\nchart_colors=[{colors}]")).is_err());
    assert_eq!(Color::parse("#FFFFFF").unwrap().rgb565(), 0xffff);
    assert_eq!(Color::parse("#FF0000").unwrap().rgb565(), 0xf800);
    assert_eq!(Color::parse("#00FF00").unwrap().rgb565(), 0x07e0);
    assert_eq!(Color::parse("#0000FF").unwrap().rgb565(), 0x001f);
}

#[test]
fn validates_http_urls_and_request_authentication() {
    for url in [
        "https://host/metrics",
        "http:///metrics",
        "http://user:password@host/metrics",
        "http://host:0/metrics",
        "http://999.999.999.999/metrics",
        "http://[::1]/metrics",
        "http://host/metrics#fragment",
        "http://host/has space",
    ] {
        let document = format!("[sources.local]\ntype='metrics'\nurl={url:?}\n{EMPTY}");
        let error = Config::parse(&document).unwrap_err();
        assert!(error.field.ends_with(".url"));
        assert!(!format!("{error}").contains(url));
    }
    let base = format!(
        "{SOURCE}\n[sources.local.auth]\ntype='basic'\nusername='private-user'\npassword='private-password'\n"
    );
    let config = Config::parse(&format!("{base}{EMPTY}")).unwrap();
    let source = &config.sources()["local"];
    let debug = format!("{config:?} {source:?} {:?}", source.auth());
    assert!(
        !debug.contains("private-user")
            && !debug.contains("private-password")
            && !debug.contains("exporter")
    );
    for auth in [
        "type='basic'\nusername='a:b'\npassword='x'",
        "type='bearer'\ntoken='bad token'",
        "type='bearer'\ntoken=''",
        "type='digest'\npassword='x'",
    ] {
        assert!(
            Config::parse(&format!("{SOURCE}\n[sources.local.auth]\n{auth}\n{EMPTY}")).is_err()
        );
    }
}

#[test]
fn rejects_header_injection_framing_duplicates_and_auth_conflicts() {
    for headers in [
        "Host='other-host'",
        "Content-Length='100'",
        "X-Token=\"secret\\r\\ninjected: yes\"",
        "'bad name'='x'",
        "X-Token='a'\nx-token='b'",
    ] {
        assert!(
            Config::parse(&format!(
                "{SOURCE}\n[sources.local.headers]\n{headers}\n{EMPTY}"
            ))
            .is_err()
        );
    }
    let conflict = format!(
        "{SOURCE}\n[sources.local.headers]\nauthorization='custom'\n[sources.local.auth]\ntype='bearer'\ntoken='secret-token'\n{EMPTY}"
    );
    assert!(Config::parse(&conflict).is_err());
    assert!(
        Config::parse(&format!(
            "{SOURCE}\n[sources.local.headers]\nX-Api-Key='secret-token'\n{EMPTY}"
        ))
        .is_ok()
    );
}

#[test]
fn source_references_and_selector_kinds_are_checked() {
    assert_eq!(
        Config::parse(&single("").replace("source='local'", "source='missing'"))
            .unwrap_err()
            .kind,
        ErrorKind::Reference
    );
    assert!(Config::parse(&single("query='up'")).is_err());
    assert!(Config::parse(&single("").replace("type='metrics'", "type='prometheus'")).is_err());
    let query = single("")
        .replace("type='metrics'", "type='prometheus'")
        .replace(
            "metric='requests_total'",
            "query='sum(rate(requests_total[5m]))'",
        );
    assert!(Config::parse(&query).is_ok());
    assert!(Config::parse(&single("").replace("requests_total", "9bad")).is_err());
    assert!(Config::parse(&single("labels={'bad-name'='value'}")).is_err());
    assert!(Config::parse(&single("labels={job='web',instance='host:9100'}")).is_ok());
}

#[test]
fn layouts_resolve_remainders_and_reject_overlap_and_invalid_spans() {
    let mut doc = format!("{SOURCE}\n[[screens]]\ntitle='Main'\nrows=2\ncolumns=3\n");
    for column in 0..3 {
        doc += &format!(
            "[[screens.widgets]]\ntype='number'\ntitle='Value'\nsource='local'\nmetric='up'\ncolumn={column}\n"
        );
    }
    let config = Config::parse(&doc).unwrap();
    let widgets = &config.screens()[0].widgets;
    assert_eq!(
        widgets[0].rect,
        Rect {
            x: 4,
            y: 4,
            width: 102,
            height: 94
        }
    );
    assert_eq!(widgets[1].rect.x, 110);
    assert_eq!(
        widgets[2].rect,
        Rect {
            x: 215,
            y: 4,
            width: 101,
            height: 94
        }
    );
    assert!(Config::parse(&doc.replace("column=1", "column=0")).is_err());
    assert!(Config::parse(&single("row_span=0")).is_err());
    assert!(Config::parse(&single("column_span=2")).is_err());
    assert!(Config::parse(&single("row=255\nrow_span=255")).is_err());
    assert!(Config::parse(&single("").replace("title='Main'", "title='Main'\ncolumns=3")).is_err());
    let span = single("column_span=2").replace("title='Main'", "title='Main'\ncolumns=2");
    assert_eq!(
        Config::parse(&span).unwrap().screens()[0].widgets[0]
            .rect
            .width,
        312
    );
}

#[test]
fn gauges_validate_ranges_thresholds_and_resolve_colors() {
    let doc=single("min=0\nmax=100\nstyle='dial'\n[[screens.widgets.thresholds]]\nvalue=70\nrole='warning'\n[[screens.widgets.thresholds]]\nvalue=90\nrole='error'\nlight='#123456'").replace("type='graph'","type='gauge'");
    let config = Config::parse(&doc).unwrap();
    let widget = &config.screens()[0].widgets[0];
    let palette = &config.theme().dark;
    assert_eq!(
        widget.gauge_color(69.99, ThemeMode::Dark, palette),
        Some(palette.accent)
    );
    assert_eq!(
        widget.gauge_color(70.0, ThemeMode::Dark, palette),
        Some(palette.warning)
    );
    assert_eq!(
        widget.gauge_color(120.0, ThemeMode::Dark, palette),
        Some(palette.error)
    );
    assert_eq!(
        widget
            .gauge_color(90.0, ThemeMode::Light, &config.theme().light)
            .unwrap()
            .rgb888(),
        0x123456
    );
    assert_eq!(widget.gauge_color(f64::NAN, ThemeMode::Dark, palette), None);
    for bad in [
        doc.replace("value=90", "value=70"),
        doc.replace("value=90", "value=nan"),
        doc.replace("max=100", "max=0"),
        doc.replace("min=0", "min=-inf"),
    ] {
        assert!(Config::parse(&bad).is_err());
    }
    assert!(Config::parse(&single("style='bar'")).is_err());
    assert!(Config::parse(&single("history_points=241")).is_err());
    assert!(Config::parse(&single("precision=0")).is_err());
    assert!(
        Config::parse(&single("max_series=2").replace("type='graph'", "type='number'")).is_err()
    );
}

fn graph_screen(metric: &str, points: usize) -> String {
    format!(
        "[[screens]]\ntitle='Graph'\n[[screens.widgets]]\ntype='graph'\ntitle='Value'\nsource='local'\nmetric='{metric}'\nhistory_points={points}\n"
    )
}

#[test]
fn history_is_shared_and_exact_capacity_boundary_is_enforced() {
    let shared = format!(
        "{SOURCE}{}{}",
        graph_screen("up", 120),
        graph_screen("up", 240)
    );
    let config = Config::parse(&shared).unwrap();
    assert_eq!(config.history_points(), 240);
    assert_eq!(config.reservations().len(), 1);
    let mut doc = SOURCE.to_string();
    for i in 0..6 {
        doc += &graph_screen(&format!("metric_{i}"), 240);
    }
    doc += &graph_screen("last", 96);
    assert_eq!(Config::parse(&doc).unwrap().history_points(), 1536);
    let error = Config::parse(&doc.replace("history_points=96", "history_points=97")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::History);
    assert_eq!(error.capacity, Some((1537, 1536)));
    assert_eq!(
        Config::parse(&single("max_series=12"))
            .unwrap()
            .history_points(),
        1440
    );
    assert!(Config::parse(&single("max_series=7\nhistory_points=240")).is_err());
}

#[test]
fn matching_labels_and_prometheus_sampling_windows_control_sharing() {
    let direct = format!(
        "{SOURCE}{}{}",
        graph_screen("up", 120),
        graph_screen("up", 120)
    );
    assert_eq!(Config::parse(&direct).unwrap().history_points(), 120);
    let different = direct.replacen("metric='up'", "metric='up'\nlabels={job='different'}", 1);
    assert_eq!(Config::parse(&different).unwrap().history_points(), 240);
    let prom = direct
        .replace("type='metrics'", "type='prometheus'")
        .replace("metric='up'", "query='up'");
    assert_eq!(Config::parse(&prom).unwrap().history_points(), 120);
    let different_window = prom.replacen("query='up'", "query='up'\nwindow_secs=3600", 1);
    assert_eq!(
        Config::parse(&different_window).unwrap().history_points(),
        240
    );
    let tracked = format!(
        "{SOURCE}{}{}",
        graph_screen("one", 1).replace("history_points=1", "history_points=1\nmax_series=12"),
        graph_screen("two", 1).replace("history_points=1", "history_points=1\nmax_series=5")
    );
    assert_eq!(
        Config::parse(&tracked).unwrap_err().capacity,
        Some((17, 16))
    );
}

#[test]
fn collection_limits_are_enforced_during_decode() {
    let mut screens = String::new();
    for _ in 0..9 {
        screens += EMPTY;
    }
    assert!(Config::parse(&screens).is_err());
    let mut sources = String::new();
    for i in 0..9 {
        sources += &format!("[sources.s{i}]\ntype='metrics'\nurl='http://host/metrics'\n");
    }
    assert!(Config::parse(&format!("{sources}{EMPTY}")).is_err());
    let mut headers = String::new();
    for i in 0..9 {
        headers += &format!("X-{i}='value'\n");
    }
    assert!(
        Config::parse(&format!(
            "{SOURCE}\n[sources.local.headers]\n{headers}{EMPTY}"
        ))
        .is_err()
    );
}

struct Reader(Result<alloc::vec::Vec<u8>, ReadError>);
impl ConfigReader for Reader {
    fn read(&mut self, filename: &str, buffer: &mut [u8]) -> Result<usize, ReadError> {
        assert_eq!(filename, "graphdis.toml");
        let bytes = self.0.as_ref().map_err(|e| *e)?;
        let count = bytes.len().min(buffer.len());
        buffer[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }
}

#[test]
fn storage_falls_back_only_when_card_or_file_is_missing() {
    let mut buffer = vec![0; MAX_CONFIG_BYTES + 1];
    for missing in [ReadError::CardMissing, ReadError::FileMissing] {
        assert_eq!(
            load(&mut Reader(Err(missing)), &mut buffer, EMPTY)
                .unwrap()
                .origin,
            ConfigOrigin::Compiled
        );
    }
    let loaded = load(
        &mut Reader(Ok(EMPTY.as_bytes().to_vec())),
        &mut buffer,
        "invalid fallback",
    )
    .unwrap();
    assert_eq!(loaded.origin, ConfigOrigin::MicroSd);
    for bad in [
        vec![],
        b"broken config".to_vec(),
        vec![0xff],
        vec![b'#'; MAX_CONFIG_BYTES + 1],
    ] {
        assert!(load(&mut Reader(Ok(bad)), &mut buffer, EMPTY).is_err());
    }
    for error in [ReadError::Io, ReadError::TooLarge] {
        assert!(load(&mut Reader(Err(error)), &mut buffer, EMPTY).is_err());
    }
    assert!(
        load(
            &mut Reader(Err(ReadError::CardMissing)),
            &mut buffer,
            "bad fallback"
        )
        .is_err()
    );
    assert!(
        load(
            &mut Reader(Err(ReadError::CardMissing)),
            &mut buffer[..10],
            EMPTY
        )
        .is_err()
    );
    let exact = format!("{EMPTY}#{}", "x".repeat(MAX_CONFIG_BYTES - EMPTY.len() - 1));
    assert_eq!(exact.len(), MAX_CONFIG_BYTES);
    assert!(load(&mut Reader(Ok(exact.into_bytes())), &mut buffer, EMPTY).is_ok());
}

#[test]
fn reload_preserves_active_state_on_failure_and_versions_success() {
    let initial = LoadedConfig {
        config: Config::parse(EMPTY).unwrap(),
        origin: ConfigOrigin::Compiled,
    };
    let mut store = ConfigStore::new(initial);
    let mut buffer = vec![0; MAX_CONFIG_BYTES + 1];
    assert!(
        store
            .reload(&mut Reader(Ok(b"invalid".to_vec())), &mut buffer, EMPTY)
            .is_err()
    );
    assert_eq!(store.generation(), 0);
    assert_eq!(store.active().config.screens()[0].title, "Setup");
    let replacement = EMPTY.replace("Setup", "Replacement");
    let outcome = store
        .reload(
            &mut Reader(Ok(replacement.into_bytes())),
            &mut buffer,
            EMPTY,
        )
        .unwrap();
    assert_eq!(
        outcome,
        Reload {
            generation: 1,
            origin: ConfigOrigin::MicroSd,
            reset_history: true
        }
    );
    assert_eq!(store.active().config.screens()[0].title, "Replacement");
    let fallback = store
        .reload(&mut Reader(Err(ReadError::FileMissing)), &mut buffer, EMPTY)
        .unwrap();
    assert_eq!(fallback.generation, 2);
    assert_eq!(fallback.origin, ConfigOrigin::Compiled);
}

#[test]
fn semantic_reload_failure_and_storage_errors_preserve_the_generation() {
    let initial = LoadedConfig {
        config: Config::parse(EMPTY).unwrap(),
        origin: ConfigOrigin::Compiled,
    };
    let mut store = ConfigStore::new(initial);
    let mut buffer = vec![0; MAX_CONFIG_BYTES + 1];
    let over_budget = single("max_series=12\nhistory_points=240");
    assert!(
        store
            .reload(
                &mut Reader(Ok(over_budget.into_bytes())),
                &mut buffer,
                EMPTY
            )
            .is_err()
    );
    assert!(
        store
            .reload(&mut Reader(Err(ReadError::Io)), &mut buffer, EMPTY)
            .is_err()
    );
    assert_eq!(store.generation(), 0);
    assert_eq!(store.active().origin, ConfigOrigin::Compiled);
    assert_eq!(store.active().config.history_points(), 0);
}

#[test]
fn diagnostics_identify_invalid_colors_without_leaking_values() {
    let bad = Config::parse(&format!("{EMPTY}\n[theme.light]\naccent='secret-color'")).unwrap_err();
    assert_eq!(bad.field, "theme.light.accent");
    assert!(bad.message.contains("#RRGGBB"));
    assert!(!format!("{bad:?} {bad}").contains("secret-color"));
    let bad = Config::parse(&format!(
        "{EMPTY}\n[theme.dark]\nchart_colors=['#FFFFFF','secret-color']"
    ))
    .unwrap_err();
    assert_eq!(bad.field, "theme.dark.chart_colors[1]");
    let bad = single("[[screens.widgets.thresholds]]\nvalue=80\nlight='secret-color'")
        .replace("type='graph'", "type='gauge'");
    assert_eq!(
        Config::parse(&bad).unwrap_err().field,
        "screens[0].widgets[0].thresholds[0].light"
    );
}

#[test]
fn scalar_limits_and_single_range_bounds_are_checked() {
    for field in [
        "poll_interval_secs=0",
        "poll_interval_secs=3601",
        "timeout_secs=0",
        "timeout_secs=121",
        "max_response_bytes=0",
        "max_response_bytes=262145",
    ] {
        assert!(Config::parse(&format!("{SOURCE}{field}\n{EMPTY}")).is_err());
    }
    for extra in [
        "precision=7",
        "window_secs=0",
        "window_secs=86401",
        "max_series=0",
        "history_points=0",
        "unit='μs'",
        "min=nan",
    ] {
        assert!(Config::parse(&single(extra)).is_err());
    }
    assert!(Config::parse(&single("min=0\nunit='µs'")).is_ok());
    assert!(Config::parse(&single("max=100\nprecision=6")).is_ok());
    assert!(Config::parse(&format!("{EMPTY}\n[display]\nrotation_secs=0")).is_err());
    assert!(Config::parse(&format!("{EMPTY}\n[display]\nrotation_secs=86401")).is_err());
    assert!(Config::parse(&format!("{EMPTY}\n[display]\nrotation_secs=86400")).is_ok());
}

#[test]
fn optional_auth_and_query_shapes_support_real_request_inputs() {
    let base = format!(
        "{SOURCE}\n[sources.local.auth]\ntype='basic'\nusername=''\npassword='api-key'\n{EMPTY}"
    );
    assert!(Config::parse(&base).is_ok());
    for token in ["=bad", "bad:token", "padding=inside"] {
        assert!(
            Config::parse(&format!(
                "{SOURCE}\n[sources.local.auth]\ntype='bearer'\ntoken='{token}'\n{EMPTY}"
            ))
            .is_err()
        );
    }
    assert!(
        Config::parse(&format!(
            "{SOURCE}\n[sources.local.auth]\ntype='bearer'\ntoken='abc_DEF.123+/=='\n{EMPTY}"
        ))
        .is_ok()
    );
    let prom = single("")
        .replace("type='metrics'", "type='prometheus'")
        .replace("metric='requests_total'", "query='''sum(\n up\n)'''");
    assert!(Config::parse(&prom).is_ok());
    let path = SOURCE.replace("/metrics", "/metrics/@tenant?token=abc");
    assert!(Config::parse(&format!("{path}{EMPTY}")).is_ok());
}

#[test]
fn limits_cover_labels_thresholds_and_device_wide_widget_count() {
    let mut labels = String::from("labels={");
    for i in 0..9 {
        labels += &format!("l{i}='v',");
    }
    labels.push('}');
    assert!(Config::parse(&single(&labels)).is_err());
    let mut thresholds = String::new();
    for i in 0..9 {
        thresholds += &format!("[[screens.widgets.thresholds]]\nvalue={i}\n");
    }
    assert!(Config::parse(&single(&thresholds).replace("type='graph'", "type='gauge'")).is_err());
    let mut doc = SOURCE.to_string();
    for screen in 0..5 {
        doc += "[[screens]]\ntitle='Numbers'\nrows=2\ncolumns=2\n";
        for index in 0..if screen == 4 { 1 } else { 4 } {
            doc += &format!(
                "[[screens.widgets]]\ntype='number'\ntitle='Value'\nsource='local'\nmetric='up'\nrow={}\ncolumn={}\n",
                index / 2,
                index % 2
            );
        }
    }
    let error = Config::parse(&doc).unwrap_err();
    assert_eq!(error.capacity, Some((17, 16)));
    assert_eq!(error.kind, ErrorKind::Limit);
}

#[test]
fn touch_calibration_is_optional_bounded_and_invertible() {
    let cfg = Config::parse(&format!(
        "{EMPTY}\n[touch]\ncalibration=[0.1,0.0,-20.0,0.0,-0.1,250.0]\n"
    ))
    .unwrap();
    assert_eq!(
        cfg.touch_calibration(),
        Some([0.1, 0.0, -20.0, 0.0, -0.1, 250.0])
    );
    for field in [
        "calibration=[0.0,0.0,0.0,0.0,0.0,0.0]",
        "calibration=[nan,0.0,0.0,0.0,1.0,0.0]",
        "calibration=[1.0,0.0,0.0,0.0,1.0,40000.0]",
        "calibration=[1.0,0.0]",
        "unknown=true",
    ] {
        assert!(Config::parse(&format!("{EMPTY}\n[touch]\n{field}\n")).is_err());
    }
}

#[test]
fn preflight_bounds_dense_invalid_documents_before_allocating_parser_trees() {
    let dense = format!("unknown=[{}]", "0,".repeat(3000));
    let error = Config::parse(&dense).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert_eq!(error.field, "document.tokens");
    let nested = format!("unknown={}0{}", "[".repeat(17), "]".repeat(17));
    let error = Config::parse(&nested).unwrap_err();
    assert_eq!(error.field, "document.nesting");
    // Strings and comments can contain brackets without increasing nesting.
    let input = format!("{}\n# {}\n", EMPTY, "[".repeat(500));
    assert!(Config::parse(&input).is_ok());
}

#[test]
fn mean_aggregation_requires_one_metrics_output_and_has_separate_reservations() {
    let raw = single("aggregate='mean'");
    assert!(Config::parse(&raw).is_ok());
    assert!(Config::parse(&single("aggregate='mean'\nmax_series=2")).is_err());
    let prom = raw
        .replace("type='metrics'", "type='prometheus'")
        .replace("metric='requests_total'", "query='up'");
    assert!(Config::parse(&prom).is_err());
}

#[test]
fn graph_legends_validate_without_changing_shared_sampling() {
    let cfg = Config::parse(&single("legend='CPU {{cpu}}'")).unwrap();
    assert_eq!(
        cfg.screens()[0].widgets[0].legend.as_deref(),
        Some("CPU {{cpu}}")
    );
    for legend in ["", "{{}}", "{{bad-name}}", "{{cpu", "\n"] {
        assert!(Config::parse(&single(&format!("legend={legend:?}"))).is_err());
    }
    assert!(
        Config::parse(&single("legend='CPU'").replace("type='graph'", "type='number'")).is_err()
    );
    let doc = format!(
        "{SOURCE}\n{}\n{}",
        graph_screen("up", 120),
        graph_screen("up", 120).replace("title='Value'", "title='Value'\nlegend='Readable'")
    );
    assert_eq!(Config::parse(&doc).unwrap().reservations().len(), 1);
}
