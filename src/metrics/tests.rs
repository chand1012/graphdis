extern crate std;
use super::*;
use crate::{
    config::Config,
    display::{Data, State},
    sources::http::{BodySink, SinkError},
};
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll},
};
use std::{boxed::Box, format, string::String};
fn run<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("pending parser"),
    }
}
fn config(prom: bool, series: u8, points: u16) -> Config {
    let source = if prom { "prometheus" } else { "metrics" };
    let selector = if prom {
        "query = \"foo\""
    } else {
        "metric = \"foo\""
    };
    Config::parse(&format!(
        r#"
version = 1
[sources.test]
type = "{source}"
url = "http://127.0.0.1/metrics"
poll_interval_secs = 1
[[screens]]
title = "Test"
[[screens.widgets]]
type = "graph"
title = "Metric"
source = "test"
{selector}
history_points = {points}
max_series = {series}
window_secs = 60
"#
    ))
    .unwrap()
}
fn buffers(cfg: &Config) -> (Model, Stage, ParserBuffers) {
    (
        Model::new(cfg, Box::leak(Box::new(empty_points()))),
        Stage::new(cfg, Box::leak(Box::new(empty_points()))),
        ParserBuffers::new(),
    )
}
fn parse(
    cfg: &Config,
    stage: &mut Stage,
    buffers: &mut ParserBuffers,
    body: &[u8],
    mode: QueryMode,
    now: u64,
    chunk: usize,
) -> Result<(), SinkError> {
    let mut sink = MetricSink::new(cfg, stage, buffers, 1, mode, now);
    sink.begin();
    for bytes in body.chunks(chunk) {
        if let Err(e) = run(sink.write(bytes)) {
            sink.abort();
            return Err(e);
        }
    }
    let out = sink.finish();
    if out.is_err() {
        sink.abort();
    }
    out
}
#[test]
fn streaming_exposition_matches_exact_filters_and_preserves_zero_and_invalid() {
    let mut raw = String::from(
        r#"
[sources.test]
type="metrics"
url="http://127.0.0.1/metrics"
[[screens]]
title="Test"
[[screens.widgets]]
type="graph"
title="Metric"
source="test"
metric="foo"
labels={method="GET"}
max_series=2
history_points=3
"#,
    );
    // A >2 KiB unselected line must not consume selected-series capacity.
    let body = format!(
        "# HELP foo A metric\nunused{{huge=\"{}\"}} 999\nfoo{{method=\"POST\"}} 8\nfoo{{path=\"/a\\\"b\",method=\"GET\"}} 0 123\nfoo{{method=\"GET\",path=\"/c\"}} NaN",
        "x".repeat(4000)
    );
    for chunk in [1, 2, 3, 7, 512, 4096] {
        let cfg = Config::parse(&raw).unwrap();
        let (mut model, mut stage, mut parser) = buffers(&cfg);
        parse(
            &cfg,
            &mut stage,
            &mut parser,
            body.as_bytes(),
            QueryMode::Metrics,
            1000,
            chunk,
        )
        .unwrap();
        model.commit(&stage, 1000);
        model.with_widget(0, |view| {
            assert_eq!(view.state, State::Ready);
            assert_eq!(view.series.len(), 2);
            assert_eq!(view.series[0].samples[0].value, 0.0);
            assert!(view.series[1].samples[0].value.is_nan());
        });
    }
    raw.clear();
}
#[test]
fn history_overwrites_oldest_and_recovers_with_a_gap() {
    let cfg = config(false, 1, 3);
    let (mut model, mut stage, mut parser) = buffers(&cfg);
    for time in 1..=5 {
        parse(
            &cfg,
            &mut stage,
            &mut parser,
            format!("foo {time}\n").as_bytes(),
            QueryMode::Metrics,
            time * 1000,
            1,
        )
        .unwrap();
        model.commit(&stage, time * 1000);
    }
    model.with_widget(0, |v| {
        assert_eq!(
            v.series[0]
                .samples
                .iter()
                .map(|s| s.value)
                .collect::<std::vec::Vec<_>>(),
            [3.0, 4.0, 5.0]
        );
    });
    model.fail(1);
    model.now_ms = 7000;
    model.with_widget(0, |v| {
        assert_eq!(v.state, State::Stale { age_secs: 2 });
        assert_eq!(v.series[0].samples.last().unwrap().value, 5.0);
    });
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        b"foo 9",
        QueryMode::Metrics,
        9000,
        1,
    )
    .unwrap();
    model.commit(&stage, 9000);
    model.with_widget(0, |v| {
        assert_eq!(v.state, State::Ready);
        assert_eq!(v.series[0].samples.len(), 3);
        assert!(v.series[0].samples[1].value.is_nan());
        assert_eq!(v.series[0].samples[2].value, 9.0);
    });
}
#[test]
fn malformed_or_excess_series_never_replace_active_data() {
    let cfg = config(false, 1, 3);
    let (mut model, mut stage, mut parser) = buffers(&cfg);
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        b"foo 42\n",
        QueryMode::Metrics,
        1000,
        1,
    )
    .unwrap();
    model.commit(&stage, 1000);
    for body in [
        "foo 11\nfoo 12\n",
        "foo{a=\"x\",a=\"y\"} 12\n",
        "foo{a=\"bad\\q\"} 1\n",
        "foo{a=\"x\"} 1\nfoo{a=\"y\"} 2\n",
        "foo nope\n",
    ] {
        assert!(
            parse(
                &cfg,
                &mut stage,
                &mut parser,
                body.as_bytes(),
                QueryMode::Metrics,
                2000,
                2
            )
            .is_err()
        );
        model.fail(1);
        model.with_widget(0, |v| {
            assert_eq!(v.series[0].samples.last().unwrap().value, 42.0)
        });
    }
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        b"other 0\n",
        QueryMode::Metrics,
        3000,
        1,
    )
    .unwrap();
    model.commit(&stage, 3000);
    model.with_widget(0, |v| assert_eq!(v.state, State::NoData));
}
#[test]
fn prometheus_vector_scalar_unicode_and_arbitrary_field_order() {
    let cfg = config(true, 2, 3);
    for body in [
        r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"__name__":"foo","instance":"caf\u00e9"},"value":[1700000000.125,"12.5"]}]}}"#,
        r#"{"data":{"result":[{"value":[1700000000.125,"12.5"],"metric":{"instance":"caf\u00e9","__name__":"foo"}}],"resultType":"vector"},"status":"success"}"#,
        r#"{"status":"success","data":{"result":[1700000000.125,"12.5"],"resultType":"scalar"}}"#,
    ] {
        for chunk in [1, 2, 7, 128] {
            let (mut model, mut stage, mut parser) = buffers(&cfg);
            parse(
                &cfg,
                &mut stage,
                &mut parser,
                body.as_bytes(),
                QueryMode::Instant { group: 0 },
                90000,
                chunk,
            )
            .unwrap();
            assert_eq!(stage.server_ms(), Some(1700000000125));
            model.commit(&stage, 90000);
            model.with_widget(0, |v| {
                assert_eq!(v.series.len(), 1);
                assert_eq!(v.series[0].samples[0].timestamp_ms, 90000);
                assert_eq!(v.series[0].samples[0].value, 12.5);
            });
        }
    }
}
#[test]
fn range_replaces_history_and_maps_server_time_to_display_time() {
    let cfg = config(true, 2, 3);
    let (mut model, mut stage, mut parser) = buffers(&cfg);
    let body=br#"{"status":"success","data":{"resultType":"matrix","result":[{"values":[[1700000000,"1"],[1700000010,"NaN"],[1700000020,"3"]],"metric":{"job":"a"}}]}}"#;
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        body,
        QueryMode::Range {
            group: 0,
            end_unix_ms: 1700000020000,
        },
        90000,
        1,
    )
    .unwrap();
    model.commit(&stage, 90000);
    model.with_widget(0, |v| {
        assert_eq!(v.series[0].samples[0].timestamp_ms, 70000);
        assert!(v.series[0].samples[1].value.is_nan());
        assert_eq!(v.series[0].samples[2].timestamp_ms, 90000);
    });
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        body,
        QueryMode::Range {
            group: 0,
            end_unix_ms: 1700000020000,
        },
        90000,
        1,
    )
    .unwrap();
    model.commit(&stage, 90000);
    model.with_widget(0, |v| assert_eq!(v.series[0].samples.len(), 3));
}
#[test]
fn rejects_json_truncation_wrong_shapes_limits_duplicate_keys_and_api_errors() {
    let cfg = config(true, 1, 3);
    let (_, mut stage, mut parser) = buffers(&cfg);
    let valid = r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1700000000,"1"]}]}}"#;
    for cut in 0..valid.len() {
        assert!(
            parse(
                &cfg,
                &mut stage,
                &mut parser,
                &valid.as_bytes()[..cut],
                QueryMode::Instant { group: 0 },
                90000,
                3
            )
            .is_err(),
            "cut {cut}"
        );
    }
    for body in [
        r#"{"status":"error","data":{"resultType":"vector","result":[]}}"#,
        r#"{"status":"success","data":{"resultType":"string","result":[1,"x"]}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":[[]]}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":null}}"#,
        r#"{"status":{},"status":"success","data":{"resultType":"vector","result":[]}}"#,
        r#"{"status":"success","status":"success","data":{"resultType":"vector","result":[]}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[01,"1"]}]}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{},"value":[1,"1",2]}]}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"a":"x","a":"y"},"value":[1,"1"]}]}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":[],}}"#,
        r#"{"status":"success","data":{"resultType":"vector","result":[]}}false"#,
    ] {
        assert!(
            parse(
                &cfg,
                &mut stage,
                &mut parser,
                body.as_bytes(),
                QueryMode::Instant { group: 0 },
                90000,
                1
            )
            .is_err(),
            "{body}"
        );
    }
    let big = valid.replace(
        "\"metric\":{}",
        &format!("\"metric\":{{\"label\":\"{}\"}}", "x".repeat(1025)),
    );
    assert!(
        parse(
            &cfg,
            &mut stage,
            &mut parser,
            big.as_bytes(),
            QueryMode::Instant { group: 0 },
            90000,
            512
        )
        .is_err()
    );
}
#[test]
fn identity_order_and_palette_slots_survive_response_reordering_and_churn() {
    let cfg = config(false, 2, 3);
    let (mut model, mut stage, mut parser) = buffers(&cfg);
    for (time, body) in [
        (1000, "foo{job=\"a\",zone=\"z\"} 1\nfoo{job=\"b\"} 2\n"),
        (2000, "foo{job=\"b\"} 3\nfoo{zone=\"z\",job=\"a\"} 4\n"),
    ] {
        parse(
            &cfg,
            &mut stage,
            &mut parser,
            body.as_bytes(),
            QueryMode::Metrics,
            time,
            1,
        )
        .unwrap();
        model.commit(&stage, time);
    }
    model.with_widget(0, |v| {
        assert_eq!(v.series[0].samples.len(), 2);
        assert_eq!(v.series[0].samples[1].value, 4.0);
        assert_eq!(v.series[1].color_slot, 1);
    });
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        b"foo{job=\"b\"} 5\n",
        QueryMode::Metrics,
        3000,
        1,
    )
    .unwrap();
    model.commit(&stage, 3000);
    model.with_widget(0, |v| {
        assert_eq!(v.series.len(), 1);
        assert_eq!(v.series[0].color_slot, 1);
        assert_eq!(v.series[0].samples.len(), 3);
    });
}
#[test]
fn affine_calibration_handles_swapped_and_inverted_axes() {
    use crate::display::touch::Calibration;
    use embedded_graphics::prelude::Point;
    let raw = [(3900, 200), (3900, 3800), (200, 200)];
    let screen = [Point::new(20, 20), Point::new(300, 20), Point::new(20, 180)];
    let c = Calibration::from_points(raw, screen).unwrap();
    for (r, s) in raw.into_iter().zip(screen) {
        let p = c.map(r);
        assert!((p.x - s.x).abs() <= 1 && (p.y - s.y).abs() <= 1);
    }
    assert!(Calibration::from_points([(1, 1); 3], screen).is_none());
}

#[test]
fn full_device_point_budget_remains_bounded_after_repeated_collection() {
    let mut raw = String::from("[sources.test]\ntype='metrics'\nurl='http://127.0.0.1/metrics'\n");
    for i in 0..7 {
        raw += &format!(
            "[[screens]]\ntitle='Graph {i}'\n[[screens.widgets]]\ntype='graph'\ntitle='Value'\nsource='test'\nmetric='value_{i}'\nhistory_points={}\n",
            if i == 6 { 96 } else { 240 }
        );
    }
    let cfg = Config::parse(&raw).unwrap();
    assert_eq!(cfg.history_points(), 1536);
    let (mut model, mut stage, mut parser) = buffers(&cfg);
    for time in 1..=300 {
        let mut body = String::new();
        for i in 0..7 {
            body += &format!("value_{i} {time}\n");
        }
        let mut sink = MetricSink::new(
            &cfg,
            &mut stage,
            &mut parser,
            127,
            QueryMode::Metrics,
            time * 1000,
        );
        sink.begin();
        run(sink.write(body.as_bytes())).unwrap();
        sink.finish().unwrap();
        model.commit(&stage, time * 1000);
    }
    let mut retained = 0;
    for i in 0..7 {
        model.with_widget(i, |v| {
            retained += v.series[0].samples.len();
            assert_eq!(v.series[0].samples.last().unwrap().value, 300.0);
        });
    }
    assert_eq!(retained, 1536);
}

#[test]
fn mean_aggregation_collapses_sixteen_cpu_cores_without_allocating_sixteen_histories() {
    let raw = "[sources.a]\ntype='metrics'\nurl='http://127.0.0.1/metrics'\n[[screens]]\ntitle='CPU'\n[[screens.widgets]]\ntype='graph'\ntitle='CPU usage'\nsource='a'\nmetric='cpu_percent'\naggregate='mean'\nhistory_points=3\n";
    let cfg = Config::parse(raw).unwrap();
    assert_eq!(cfg.tracked_series(), 1);
    assert_eq!(cfg.history_points(), 3);
    let (mut model, mut stage, mut parser) = buffers(&cfg);
    let mut body = String::new();
    for core in 0..16 {
        body += &format!("cpu_percent{{core=\"{core}\"}} {}\n", core * 2);
    }
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        body.as_bytes(),
        QueryMode::Metrics,
        1000,
        1,
    )
    .unwrap();
    model.commit(&stage, 1000);
    model.with_widget(0, |v| {
        assert_eq!(v.series.len(), 1);
        assert!((v.series[0].samples[0].value - 15.0).abs() < 1e-12);
    });
    assert!(
        parse(
            &cfg,
            &mut stage,
            &mut parser,
            b"cpu_percent{core=\"0\"} 0\ncpu_percent{core=\"0\"} 0\n",
            QueryMode::Metrics,
            2000,
            1
        )
        .is_err()
    );
    let too_many = (0..257)
        .map(|i| format!("cpu_percent{{core=\"{i}\"}} 1\n"))
        .collect::<String>();
    assert!(
        parse(
            &cfg,
            &mut stage,
            &mut parser,
            too_many.as_bytes(),
            QueryMode::Metrics,
            2000,
            1
        )
        .is_err()
    );
    parse(
        &cfg,
        &mut stage,
        &mut parser,
        b"cpu_percent{core=\"0\"} NaN\n",
        QueryMode::Metrics,
        3000,
        1,
    )
    .unwrap();
    model.commit(&stage, 3000);
    model.with_widget(0, |v| {
        assert!(v.series[0].samples.last().unwrap().value.is_nan())
    });
}
