extern crate std;

use super::*;
use crate::config::Config;
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::{
    cell::{Cell, RefCell},
    pin::pin,
    task::{Context, Poll},
};
use std::task::Waker;
fn waker() -> Waker {
    Waker::noop().clone()
}
fn run<F: Future>(future: F) -> F::Output {
    let waker = waker();
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending mock operation"),
    }
}

struct Clock {
    expire: bool,
    calls: Cell<usize>,
    seconds: Cell<u32>,
}
impl Clock {
    fn new(expire: bool) -> Self {
        Self {
            expire,
            calls: Cell::new(0),
            seconds: Cell::new(0),
        }
    }
}
impl Deadline for Clock {
    async fn run<F: Future>(&self, seconds: u32, operation: F) -> Result<F::Output, TimedOut> {
        self.calls.set(self.calls.get() + 1);
        self.seconds.set(seconds);
        let mut operation = pin!(operation);
        core::future::poll_fn(|cx| match operation.as_mut().poll(cx) {
            Poll::Ready(value) => Poll::Ready(Ok(value)),
            Poll::Pending if self.expire => Poll::Ready(Err(TimedOut)),
            Poll::Pending => Poll::Pending,
        })
        .await
    }
}

struct Wire {
    response: Vec<u8>,
    sent: RefCell<Vec<u8>>,
    max_read: usize,
    max_write: usize,
    stall_at: Option<usize>,
    fail_at: Option<usize>,
    dns_fail: bool,
    connect_fail: bool,
    opened: Cell<usize>,
    closed: Cell<usize>,
    resolved: RefCell<Vec<String>>,
    remote: Cell<Option<SocketAddr>>,
}
impl Wire {
    fn new(response: impl AsRef<[u8]>) -> Self {
        Self {
            response: response.as_ref().to_vec(),
            sent: RefCell::new(Vec::new()),
            max_read: usize::MAX,
            max_write: usize::MAX,
            stall_at: None,
            fail_at: None,
            dns_fail: false,
            connect_fail: false,
            opened: Cell::new(0),
            closed: Cell::new(0),
            resolved: RefCell::new(Vec::new()),
            remote: Cell::new(None),
        }
    }
    fn text(&self) -> String {
        String::from_utf8(self.sent.borrow().clone()).unwrap()
    }
}
struct Tcp<'a>(&'a Wire);
struct DnsMock<'a>(&'a Wire);
struct Connection<'a> {
    wire: &'a Wire,
    pos: usize,
}
impl embedded_io::ErrorType for Connection<'_> {
    type Error = embedded_io::ErrorKind;
}
impl embedded_io_async::Read for Connection<'_> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        if self.wire.stall_at.is_some_and(|at| self.pos >= at) {
            return core::future::pending().await;
        }
        if self.wire.fail_at.is_some_and(|at| self.pos >= at) {
            return Err(embedded_io::ErrorKind::ConnectionReset);
        }
        let end = self
            .wire
            .response
            .len()
            .min(self.wire.stall_at.unwrap_or(usize::MAX))
            .min(self.wire.fail_at.unwrap_or(usize::MAX));
        let size = (end - self.pos).min(out.len()).min(self.wire.max_read);
        out[..size].copy_from_slice(&self.wire.response[self.pos..self.pos + size]);
        self.pos += size;
        Ok(size)
    }
}
impl embedded_io_async::Write for Connection<'_> {
    async fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        let size = data.len().min(self.wire.max_write);
        self.wire.sent.borrow_mut().extend_from_slice(&data[..size]);
        Ok(size)
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
impl Drop for Connection<'_> {
    fn drop(&mut self) {
        self.wire.closed.set(self.wire.closed.get() + 1);
    }
}
impl TcpConnect for Tcp<'_> {
    type Error = embedded_io::ErrorKind;
    type Connection<'a>
        = Connection<'a>
    where
        Self: 'a;
    async fn connect<'a>(&'a self, remote: SocketAddr) -> Result<Connection<'a>, Self::Error> {
        self.0.remote.set(Some(remote));
        if self.0.connect_fail {
            return Err(embedded_io::ErrorKind::ConnectionRefused);
        }
        self.0.opened.set(self.0.opened.get() + 1);
        Ok(Connection {
            wire: self.0,
            pos: 0,
        })
    }
}
impl Dns for DnsMock<'_> {
    type Error = ();
    async fn get_host_by_name(
        &self,
        host: &str,
        kind: embedded_nal_async::AddrType,
    ) -> Result<IpAddr, ()> {
        assert_eq!(kind, embedded_nal_async::AddrType::IPv4);
        self.0.resolved.borrow_mut().push(host.to_string());
        if self.0.dns_fail {
            Err(())
        } else {
            Ok(IpAddr::V4(core::net::Ipv4Addr::LOCALHOST))
        }
    }
    async fn get_host_by_address(&self, _: IpAddr, _: &mut [u8]) -> Result<usize, ()> {
        Err(())
    }
}

#[derive(Default)]
struct Sink {
    data: Vec<u8>,
    committed: bool,
    aborts: usize,
    writes: usize,
    max_chunk: usize,
    reject: bool,
    finish_fail: bool,
    stall: bool,
}
impl BodySink for Sink {
    fn begin(&mut self) {
        self.data.clear();
        self.committed = false;
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<(), SinkError> {
        self.writes += 1;
        self.max_chunk = self.max_chunk.max(bytes.len());
        if self.stall {
            return core::future::pending().await;
        }
        if self.reject {
            return Err(SinkError);
        }
        self.data.extend_from_slice(bytes);
        Ok(())
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        if self.finish_fail {
            return Err(SinkError);
        }
        self.committed = true;
        Ok(())
    }
    fn abort(&mut self) {
        self.aborts += 1;
        self.data.clear();
        self.committed = false;
    }
}

fn config(kind: &str, extra: &str) -> Config {
    Config::parse(&format!("[sources.test]\ntype='{kind}'\nurl='http://exporter:9100/proxy/'\n{extra}\n[[screens]]\ntitle='Setup'\n")).unwrap()
}
fn fetch(
    wire: &Wire,
    source: &Source,
    request: Request<'_>,
    sink: &mut impl BodySink,
) -> Result<Summary, Error> {
    run(
        Client::new(&Tcp(wire), &DnsMock(wire), &Clock::new(false)).fetch(
            source,
            request,
            &mut HttpBuffers::new(),
            sink,
        ),
    )
}

#[test]
fn direct_get_applies_headers_port_auth_and_identity_with_partial_io() {
    let config = config(
        "metrics",
        "[sources.test.headers]\nX-Api-Key='private-key'\n[sources.test.auth]\ntype='basic'\nusername='user'\npassword='password'",
    );
    let mut wire = Wire::new(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: 5\r\n\r\nup 1\n",
    );
    wire.max_read = 3;
    wire.max_write = 2;
    let mut sink = Sink::default();
    let summary = fetch(
        &wire,
        &config.sources()["test"],
        Request::Metrics,
        &mut sink,
    )
    .unwrap();
    assert_eq!(
        summary,
        Summary {
            status: 200,
            content_type: ContentKind::Text,
            body_bytes: 5
        }
    );
    assert_eq!(sink.data, b"up 1\n");
    assert!(sink.committed);
    assert_eq!(wire.closed.get(), 1);
    let request = wire.text();
    assert!(request.starts_with("GET /proxy/ HTTP/1.1\r\n"));
    for header in [
        "Host: exporter:9100\r\n",
        "X-Api-Key: private-key\r\n",
        "Authorization: Basic dXNlcjpwYXNzd29yZA==\r\n",
        "Accept-Encoding: identity\r\n",
        "Connection: close\r\n",
    ] {
        assert!(request.contains(header), "missing expected request header");
    }
    assert_eq!(wire.resolved.borrow().as_slice(), ["exporter"]);
}

#[test]
fn supports_maximum_basic_credentials_and_all_eight_custom_headers() {
    let mut extra = String::from("[sources.test.headers]\n");
    for index in 0..8 {
        extra += &format!("X-{index}='value'\n");
    }
    extra += &format!(
        "[sources.test.auth]\ntype='basic'\nusername='{}'\npassword='{}'",
        "u".repeat(128),
        "p".repeat(256)
    );
    let config = config("metrics", &extra);
    let wire = Wire::new("HTTP/1.1 204 No Content\r\n\r\n");
    fetch(
        &wire,
        &config.sources()["test"],
        Request::Metrics,
        &mut Sink::default(),
    )
    .unwrap();
    let text = wire.text();
    let auth = text
        .lines()
        .find(|line| line.starts_with("Authorization:"))
        .unwrap();
    assert_eq!(
        auth.strip_prefix("Authorization: Basic ").unwrap().len(),
        516
    );
}

#[test]
fn prometheus_queries_are_percent_encoded_and_reverse_proxy_paths_preserved() {
    let config = config(
        "prometheus",
        "[sources.test.auth]\ntype='bearer'\ntoken='private-token'",
    );
    let wire = Wire::new(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}",
    );
    let query = "sum(up{job=\"a&b\"}) + µ";
    let result = fetch(
        &wire,
        &config.sources()["test"],
        Request::Instant {
            query,
            time_ms: Some(12345),
        },
        &mut Sink::default(),
    )
    .unwrap();
    assert_eq!(result.content_type, ContentKind::Json);
    let request = wire.text();
    assert!(request.starts_with("GET /proxy/api/v1/query?query=sum%28up%7Bjob%3D%22a%26b%22%7D%29%20%2B%20%C2%B5&time=12.345 HTTP/1.1\r\n"));
    assert!(request.contains("Authorization: Bearer private-token\r\n"));
    let range = Wire::new("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    fetch(
        &range,
        &config.sources()["test"],
        Request::Range {
            query: "up",
            start_ms: 1000,
            end_ms: 2000,
            step_ms: 500,
        },
        &mut Sink::default(),
    )
    .unwrap();
    assert!(range.text().starts_with(
        "GET /proxy/api/v1/query_range?query=up&start=1.000&end=2.000&step=0.500 HTTP/1.1"
    ));
}

#[test]
fn rejects_invalid_requests_before_connecting_and_redacts_debug() {
    let metrics = config("metrics", "");
    let prom = config("prometheus", "");
    let wire = Wire::new("");
    assert_eq!(
        fetch(
            &wire,
            &metrics.sources()["test"],
            Request::Instant {
                query: "private-query",
                time_ms: None
            },
            &mut Sink::default()
        ),
        Err(Error::InvalidRequest)
    );
    for (start, end, step) in [(0, 1000, 0), (2000, 1000, 1), (0, 240, 1)] {
        assert_eq!(
            fetch(
                &wire,
                &prom.sources()["test"],
                Request::Range {
                    query: "up",
                    start_ms: start,
                    end_ms: end,
                    step_ms: step
                },
                &mut Sink::default()
            ),
            Err(Error::InvalidRequest)
        );
    }
    let mut buffers = HttpBuffers::new();
    request::prepare(
        &prom.sources()["test"],
        Request::Instant {
            query: "private-query",
            time_ms: None,
        },
        &mut buffers.request,
    )
    .unwrap();
    assert!(
        !format!(
            "{buffers:?} {:?}",
            Request::Instant {
                query: "private-query",
                time_ms: None
            }
        )
        .contains("private-query")
    );
    assert_eq!(wire.opened.get(), 0);
}

#[test]
fn streams_large_bodies_with_a_fixed_chunk_size() {
    let config = config("metrics", "");
    let data = vec![b'x'; 16000];
    let mut response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", data.len()).into_bytes();
    response.extend_from_slice(&data);
    let wire = Wire::new(response);
    let mut sink = Sink::default();
    assert_eq!(
        fetch(
            &wire,
            &config.sources()["test"],
            Request::Metrics,
            &mut sink
        )
        .unwrap()
        .body_bytes,
        16000
    );
    assert_eq!(sink.data, data);
    assert!(sink.writes > 1);
    assert!(sink.max_chunk <= CHUNK_BYTES);
    assert!(core::mem::size_of::<HttpBuffers>() <= 6400);
}

#[test]
fn chunked_extensions_trailers_and_case_insensitive_transfer_coding() {
    let config = config("metrics", "");
    let mut wire = Wire::new(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: ChUnKeD\r\nContent-Type: application/openmetrics-text; version=1.0.0\r\n\r\n3;name=value\r\nup \r\n2\r\n1\n\r\n0\r\nX-Checksum: ignored\r\n\r\n",
    );
    wire.max_read = 1;
    let mut sink = Sink::default();
    let summary = fetch(
        &wire,
        &config.sources()["test"],
        Request::Metrics,
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.data, b"up 1\n");
    assert_eq!(summary.content_type, ContentKind::OpenMetrics);
    assert_eq!(wire.closed.get(), 1);
}

#[test]
fn exact_limits_work_for_fixed_chunked_and_connection_close_framing() {
    let config = config("metrics", "max_response_bytes=4");
    for response in [
        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndata",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\ndata\r\n0\r\n\r\n",
        "HTTP/1.0 200 OK\r\n\r\ndata",
    ] {
        let wire = Wire::new(response);
        let mut sink = Sink::default();
        assert_eq!(
            fetch(
                &wire,
                &config.sources()["test"],
                Request::Metrics,
                &mut sink
            )
            .unwrap()
            .body_bytes,
            4
        );
        assert_eq!(sink.data, b"data");
    }
    for response in [
        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n12345\r\n0\r\n\r\n",
        "HTTP/1.0 200 OK\r\n\r\n12345",
    ] {
        let wire = Wire::new(response);
        let mut sink = Sink::default();
        assert_eq!(
            fetch(
                &wire,
                &config.sources()["test"],
                Request::Metrics,
                &mut sink
            ),
            Err(Error::ResponseTooLarge)
        );
        assert!(sink.data.is_empty());
        assert_eq!(sink.aborts, 1);
        assert_eq!(wire.closed.get(), 1);
    }
}

#[test]
fn truncation_and_invalid_framing_never_commit_partial_data() {
    let config = config("metrics", "");
    for response in [
        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n123",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n123",
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n123\r\n0\r\n",
    ] {
        let mut wire = Wire::new(response);
        wire.max_read = 2;
        let mut sink = Sink::default();
        assert_eq!(
            fetch(
                &wire,
                &config.sources()["test"],
                Request::Metrics,
                &mut sink
            ),
            Err(Error::Truncated)
        );
        assert!(sink.data.is_empty());
        assert!(!sink.committed);
        assert_eq!(wire.closed.get(), 1);
    }
    for headers in [
        "Content-Length: 0\r\nContent-Length: 0\r\n",
        "Content-Length: 0\r\nTransfer-Encoding: chunked\r\n",
        "Content-Length: +2\r\n",
    ] {
        assert_eq!(
            fetch(
                &Wire::new(format!("HTTP/1.1 200 OK\r\n{headers}\r\n")),
                &config.sources()["test"],
                Request::Metrics,
                &mut Sink::default()
            ),
            Err(Error::Protocol)
        );
    }
    assert_eq!(
        fetch(
            &Wire::new("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nZ\r\n"),
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::Protocol)
    );
}

#[test]
fn redirects_errors_partial_responses_and_compression_are_rejected() {
    let config = config("metrics", "");
    for status in [301, 302, 401, 403, 404, 429, 500, 206] {
        let wire = Wire::new(format!(
            "HTTP/1.1 {status} Status\r\nLocation: http://other/private\r\nContent-Length: 0\r\n\r\n"
        ));
        let mut sink = Sink::default();
        assert_eq!(
            fetch(
                &wire,
                &config.sources()["test"],
                Request::Metrics,
                &mut sink
            ),
            Err(Error::Status(status))
        );
        assert_eq!(wire.opened.get(), 1);
        assert_eq!(wire.closed.get(), 1);
        assert_eq!(sink.aborts, 1);
    }
    for coding in ["Content-Encoding: gzip", "Transfer-Encoding: gzip, chunked"] {
        assert_eq!(
            fetch(
                &Wire::new(format!("HTTP/1.1 200 OK\r\n{coding}\r\n\r\n")),
                &config.sources()["test"],
                Request::Metrics,
                &mut Sink::default()
            ),
            Err(Error::UnsupportedEncoding)
        );
    }
}

#[test]
fn bounded_headers_and_informational_responses() {
    let config = config("metrics", "");
    let mut oversized = String::from("HTTP/1.1 200 OK\r\nX-Data: ");
    oversized += &"x".repeat(HEADER_BYTES);
    assert_eq!(
        fetch(
            &Wire::new(oversized),
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::HeadersTooLarge)
    );
    let wire = Wire::new(
        "HTTP/1.1 103 Early Hints\r\nLink: ignored\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n",
    );
    assert_eq!(
        fetch(
            &wire,
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        )
        .unwrap()
        .status,
        204
    );
    assert_eq!(
        fetch(
            &Wire::new("HTTP/1.1 101 Switching Protocols\r\n\r\n"),
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::Protocol)
    );
}

#[test]
fn deadlines_cover_stalled_bodies_and_consumers_and_abort_on_cancellation() {
    let config = config("metrics", "timeout_secs=7");
    let mut wire = Wire::new("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345");
    wire.stall_at = Some(wire.response.len() - 2);
    let clock = Clock::new(true);
    let mut sink = Sink::default();
    let mut buffers = HttpBuffers::new();
    assert_eq!(
        run(Client::new(&Tcp(&wire), &DnsMock(&wire), &clock).fetch(
            &config.sources()["test"],
            Request::Metrics,
            &mut buffers,
            &mut sink
        )),
        Err(Error::Timeout)
    );
    assert_eq!(clock.calls.get(), 1);
    assert_eq!(clock.seconds.get(), 7);
    assert_eq!(wire.closed.get(), 1);
    assert_eq!(sink.aborts, 1);
    assert!(sink.data.is_empty());
    let wire = Wire::new("HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx");
    let tcp = Tcp(&wire);
    let dns = DnsMock(&wire);
    let clock = Clock::new(false);
    let client = Client::new(&tcp, &dns, &clock);
    let mut sink = Sink {
        stall: true,
        ..Default::default()
    };
    {
        let future = client.fetch(
            &config.sources()["test"],
            Request::Metrics,
            &mut buffers,
            &mut sink,
        );
        let mut future = pin!(future);
        let waker = waker();
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
    }
    assert_eq!(sink.aborts, 1);
    assert_eq!(wire.closed.get(), 1);
    let mut sink = Sink {
        stall: true,
        ..Default::default()
    };
    assert_eq!(
        run(Client::new(&tcp, &dns, &Clock::new(true)).fetch(
            &config.sources()["test"],
            Request::Metrics,
            &mut buffers,
            &mut sink
        )),
        Err(Error::Timeout)
    );
    assert_eq!(sink.aborts, 1);
}

#[test]
fn consumer_failures_and_network_failures_release_resources() {
    let config = config("metrics", "");
    let wire = Wire::new("HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx");
    for finish_fail in [true, false] {
        let mut sink = Sink {
            finish_fail,
            reject: !finish_fail,
            ..Default::default()
        };
        assert_eq!(
            fetch(
                &wire,
                &config.sources()["test"],
                Request::Metrics,
                &mut sink
            ),
            Err(Error::Consumer)
        );
        assert_eq!(sink.aborts, 1);
    }
    assert_eq!(wire.closed.get(), 2);
    let mut failing = Wire::new("");
    failing.dns_fail = true;
    assert_eq!(
        fetch(
            &failing,
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::Dns)
    );
    assert_eq!(failing.opened.get(), 0);
    failing.dns_fail = false;
    failing.connect_fail = true;
    assert_eq!(
        fetch(
            &failing,
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::Transport(embedded_io::ErrorKind::ConnectionRefused))
    );
    let mut short = [0; 0];
    let mut sink = SliceSink::new(&mut short);
    assert_eq!(
        fetch(
            &wire,
            &config.sources()["test"],
            Request::Metrics,
            &mut sink
        ),
        Err(Error::Consumer)
    );
    assert!(sink.bytes().is_empty());
}

#[test]
fn numeric_hosts_skip_dns_and_direct_query_parameters_are_unchanged() {
    let doc = "[sources.test]\ntype='metrics'\nurl='http://192.168.1.20:8080/metrics?token=private-token'\n[[screens]]\ntitle='Setup'";
    let config = Config::parse(doc).unwrap();
    let wire = Wire::new("HTTP/1.1 204 No Content\r\n\r\n");
    fetch(
        &wire,
        &config.sources()["test"],
        Request::Metrics,
        &mut Sink::default(),
    )
    .unwrap();
    assert!(wire.resolved.borrow().is_empty());
    assert_eq!(wire.remote.get().unwrap().port(), 8080);
    assert!(
        wire.text()
            .starts_with("GET /metrics?token=private-token HTTP/1.1\r\n")
    );
}

#[test]
fn transport_errors_abort_staging_and_unknown_binary_headers_are_ignored() {
    let config = config("metrics", "");
    let mut wire = Wire::new("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n12345");
    wire.fail_at = Some(wire.response.len() - 2);
    let mut sink = Sink::default();
    assert_eq!(
        fetch(
            &wire,
            &config.sources()["test"],
            Request::Metrics,
            &mut sink
        ),
        Err(Error::Transport(embedded_io::ErrorKind::ConnectionReset))
    );
    assert_eq!(sink.aborts, 1);
    assert!(sink.data.is_empty());
    assert_eq!(wire.closed.get(), 1);
    let wire = Wire::new(b"HTTP/1.1 200 OK\r\nX-Unknown: \xff\r\nContent-Length: 0\r\n\r\n");
    assert!(
        fetch(
            &wire,
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        )
        .is_ok()
    );
}

#[test]
fn excessive_response_fields_and_invalid_trailers_are_bounded() {
    let config = config("metrics", "");
    let mut response = String::from("HTTP/1.1 200 OK\r\n");
    for index in 0..65 {
        response += &format!("X-{index}: x\r\n");
    }
    response += "\r\n";
    assert_eq!(
        fetch(
            &Wire::new(response),
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::HeadersTooLarge)
    );
    let prefix = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n";
    for trailer in ["Content-Length: 0\r\n\r\n", "invalid\r\n\r\n"] {
        assert_eq!(
            fetch(
                &Wire::new(format!("{prefix}{trailer}")),
                &config.sources()["test"],
                Request::Metrics,
                &mut Sink::default()
            ),
            Err(Error::Protocol)
        );
    }
    let mut response = prefix.to_string();
    for index in 0..17 {
        response += &format!("X-{index}: x\r\n");
    }
    response += "\r\n";
    assert_eq!(
        fetch(
            &Wire::new(response),
            &config.sources()["test"],
            Request::Metrics,
            &mut Sink::default()
        ),
        Err(Error::HeadersTooLarge)
    );
}

#[cfg(feature = "metrics")]
#[test]
fn streaming_http_metrics_publish_render_and_preserve_readings_after_failure() {
    use crate::{
        display::{self, Data, Navigation, State},
        metrics::{MetricSink, Model, ParserBuffers, QueryMode, Stage, empty_points},
    };
    use alloc::boxed::Box;
    use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
    let cfg=Config::parse("[sources.test]\ntype='metrics'\nurl='http://exporter:9100/metrics'\n[[screens]]\ntitle='Live'\n[[screens.widgets]]\ntype='number'\ntitle='Up'\nsource='test'\nmetric='up'\n").unwrap();
    let mut model = Model::new(&cfg, Box::leak(Box::new(empty_points())));
    let mut stage = Stage::new(&cfg, Box::leak(Box::new(empty_points())));
    let mut parser = ParserBuffers::new();
    let mut http = HttpBuffers::new();
    let body = format!("{}up 1\n", "unused 9\n".repeat(2000));
    let mut response = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    for chunk in body.as_bytes().chunks(113) {
        response += &format!(
            "{:x}\r\n{}\r\n",
            chunk.len(),
            core::str::from_utf8(chunk).unwrap()
        );
    }
    response += "0\r\n\r\n";
    let mut wire = Wire::new(response);
    wire.max_read = 7;
    let mut sink = MetricSink::new(&cfg, &mut stage, &mut parser, 1, QueryMode::Metrics, 1000);
    run(
        Client::new(&Tcp(&wire), &DnsMock(&wire), &Clock::new(false)).fetch(
            &cfg.sources()["test"],
            Request::Metrics,
            &mut http,
            &mut sink,
        ),
    )
    .unwrap();
    model.commit(&stage, 1000);
    model.with_widget(0, |view| {
        assert_eq!(view.state, State::Ready);
        assert_eq!(view.series[0].samples[0].value, 1.0);
    });
    struct Panel(usize);
    impl Dimensions for Panel {
        fn bounding_box(&self) -> Rectangle {
            Rectangle::new(Point::zero(), Size::new(320, 240))
        }
    }
    impl DrawTarget for Panel {
        type Color = Rgb565;
        type Error = core::convert::Infallible;
        fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
            &mut self,
            pixels: I,
        ) -> Result<(), Self::Error> {
            for Pixel(p, _) in pixels {
                assert!((0..320).contains(&p.x) && (0..240).contains(&p.y));
                self.0 += 1;
            }
            Ok(())
        }
    }
    let mut panel = Panel(0);
    display::render(&mut panel, &cfg, &Navigation::new(1, None, 0), &model, 1000).unwrap();
    assert!(panel.0 > 320 * 240);
    for response in [
        "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nup 2\nup 3\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nup 9\n",
    ] {
        let wire = Wire::new(response);
        let mut sink = MetricSink::new(&cfg, &mut stage, &mut parser, 1, QueryMode::Metrics, 2000);
        assert!(
            run(
                Client::new(&Tcp(&wire), &DnsMock(&wire), &Clock::new(false)).fetch(
                    &cfg.sources()["test"],
                    Request::Metrics,
                    &mut http,
                    &mut sink
                )
            )
            .is_err()
        );
        model.fail(1);
        model.now_ms = 2000;
        model.with_widget(0, |v| {
            assert_eq!(v.state, State::Stale { age_secs: 1 });
            assert_eq!(v.series[0].samples[0].value, 1.0);
        });
    }
}
