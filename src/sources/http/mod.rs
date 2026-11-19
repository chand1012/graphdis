//! Allocation-free HTTP GET transport for metrics and Prometheus APIs.
//!
//! One caller-owned buffer set per in-flight request; no redirects, retries,
//! compression, or metric parsing. All errors and Debug output are secret-free.

mod request;
mod response;
mod sink;

pub use request::Request;
pub use sink::{BodySink, SliceSink};

use crate::config::{Source, SourceKind};
use core::{
    fmt,
    future::Future,
    net::{IpAddr, SocketAddr},
};
use embedded_io::Error as _;
use embedded_io_async::Write;
use embedded_nal_async::{Dns, TcpConnect};
use reqwless::request::RequestBuilder;

/// Fixed response-header limit, in bytes, for each header block.
pub const HEADER_BYTES: usize = 2048;
/// Largest decoded chunk forwarded to a consumer, in bytes.
pub const CHUNK_BYTES: usize = 512;

struct RequestBuffers {
    url: heapless::String<2304>,
    host: heapless::String<260>,
    authorization: heapless::String<522>,
    basic_plain: [u8; 385],
}

/// Reusable request/response scratch, approximately 6.3 KiB plus metadata.
/// Initialize statically/in place on ESP32; do not place it in a small task stack.
pub struct HttpBuffers {
    request: RequestBuffers,
    response: ResponseBuffers,
}
struct ResponseBuffers {
    headers: [u8; HEADER_BYTES],
    chunk: [u8; CHUNK_BYTES],
    line: [u8; 256],
}

impl HttpBuffers {
    /// Empty fixed-capacity buffers. Does not allocate.
    pub const fn new() -> Self {
        Self {
            request: RequestBuffers {
                url: heapless::String::new(),
                host: heapless::String::new(),
                authorization: heapless::String::new(),
                basic_plain: [0; 385],
            },
            response: ResponseBuffers {
                headers: [0; HEADER_BYTES],
                chunk: [0; CHUNK_BYTES],
                line: [0; 256],
            },
        }
    }
}
impl Default for HttpBuffers {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for HttpBuffers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HttpBuffers([REDACTED])")
    }
}

/// Sanitized HTTP/transport failure; contains no URL, body, header, or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Invalid source settings or request/source mismatch.
    InvalidRequest,
    /// Encoded request exceeds its fixed buffer.
    RequestTooLarge,
    /// Overall request deadline expired, including DNS, connect, I/O, and sink writes.
    Timeout,
    /// Name lookup failed or returned an unsupported address family.
    Dns,
    /// Transport failure category only, with no underlying private message.
    Transport(embedded_io::ErrorKind),
    /// Unacceptable HTTP status (including redirects and unexpected partial responses).
    Status(u16),
    /// Malformed or ambiguous response framing.
    Protocol,
    /// Response header block, chunk line, or trailer limit exceeded.
    HeadersTooLarge,
    /// Decoded body exceeds the configured source limit.
    ResponseTooLarge,
    /// Peer closed before the declared body or framing completed.
    Truncated,
    /// Compressed content or unsupported transfer/content coding.
    UnsupportedEncoding,
    /// The consumer rejected data or could not finish its transaction.
    Consumer,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl core::error::Error for Error {}

/// A consumer rejected the current transaction; private parser details stay in the sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SinkError;
/// Deadline expiry marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedOut;

/// Clock adapter, allowing transport and timeout behavior to be tested on the host.
#[allow(async_fn_in_trait)]
pub trait Deadline {
    /// Poll one operation under a single deadline; cancellation must drop that future.
    async fn run<F: Future>(&self, seconds: u32, operation: F) -> Result<F::Output, TimedOut>;
}

/// Embassy deadline used by ESP32 network clients.
#[cfg(feature = "firmware")]
pub struct EmbassyDeadline;
#[cfg(feature = "firmware")]
impl Deadline for EmbassyDeadline {
    async fn run<F: Future>(&self, seconds: u32, operation: F) -> Result<F::Output, TimedOut> {
        embassy_time::with_timeout(
            embassy_time::Duration::from_secs(u64::from(seconds)),
            operation,
        )
        .await
        .map_err(|_| TimedOut)
    }
}

/// Recognized MIME family, ignoring parameters; bodies are not decoded as metrics here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    /// Plain-text metrics.
    Text,
    /// OpenMetrics exposition.
    OpenMetrics,
    /// Prometheus JSON.
    Json,
    /// Absent or unrecognized content type.
    Unknown,
}

/// Safe metadata returned only after a complete successful response and sink commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Successful HTTP status.
    pub status: u16,
    /// MIME family, not raw header values.
    pub content_type: ContentKind,
    /// Decoded body bytes delivered.
    pub body_bytes: usize,
}

/// HTTP transport borrowing a TCP pool, DNS resolver, and clock. No shared locks.
pub struct Client<'a, T, D, C> {
    tcp: &'a T,
    dns: &'a D,
    clock: &'a C,
}
impl<'a, T: TcpConnect, D: Dns, C: Deadline> Client<'a, T, D, C> {
    /// Borrow resources; connection ownership remains per request.
    pub fn new(tcp: &'a T, dns: &'a D, clock: &'a C) -> Self {
        Self { tcp, dns, clock }
    }

    /// Execute one HTTP GET under the source's overall deadline.
    ///
    /// Sink staging is aborted on every failure and when this future is dropped.
    /// Connections are dropped after every response; redirects are never followed.
    /// Synchronous sink finalization must remain bounded and publish only on success.
    pub async fn fetch<S: BodySink>(
        &self,
        source: &Source,
        request: Request<'_>,
        buffers: &mut HttpBuffers,
        sink: &mut S,
    ) -> Result<Summary, Error> {
        let mut transaction = sink::Transaction::new(sink);
        let operation = self.fetch_inner(source, request, buffers, transaction.sink);
        let summary = self
            .clock
            .run(source.timeout_secs(), operation)
            .await
            .map_err(|_| Error::Timeout)??;
        transaction.commit()?;
        Ok(summary)
    }

    async fn fetch_inner<S: BodySink>(
        &self,
        source: &Source,
        request: Request<'_>,
        buffers: &mut HttpBuffers,
        sink: &mut S,
    ) -> Result<Summary, Error> {
        request::prepare(source, request, &mut buffers.request)?;
        let mut extra: heapless::Vec<(&str, &str), 12> = heapless::Vec::new();
        let mut accept = false;
        let mut encoding = false;
        for (name, value) in source.headers() {
            accept |= name.eq_ignore_ascii_case("accept");
            if name.eq_ignore_ascii_case("accept-encoding") {
                if !value.expose().trim().eq_ignore_ascii_case("identity") {
                    return Err(Error::UnsupportedEncoding);
                }
                encoding = true;
            }
            extra
                .push((name.as_str(), value.expose()))
                .map_err(|_| Error::RequestTooLarge)?;
        }
        if !buffers.request.authorization.is_empty() {
            extra
                .push(("Authorization", buffers.request.authorization.as_str()))
                .map_err(|_| Error::RequestTooLarge)?;
        }
        if !accept {
            extra.push(("Accept",match source.kind(){SourceKind::Metrics=>"text/plain;version=0.0.4, application/openmetrics-text;version=1.0.0;q=0.9",SourceKind::Prometheus=>"application/json"})).map_err(|_|Error::RequestTooLarge)?;
        }
        if !encoding {
            extra
                .push(("Accept-Encoding", "identity"))
                .map_err(|_| Error::RequestTooLarge)?;
        }
        extra
            .push(("Connection", "close"))
            .map_err(|_| Error::RequestTooLarge)?;
        let url = nourl::Url::parse(&buffers.request.url).map_err(|_| Error::InvalidRequest)?;
        let address = match url.host().parse::<core::net::Ipv4Addr>() {
            Ok(address) => IpAddr::V4(address),
            Err(_) => self
                .dns
                .get_host_by_name(url.host(), embedded_nal_async::AddrType::IPv4)
                .await
                .map_err(|_| Error::Dns)?,
        };
        if !address.is_ipv4() {
            return Err(Error::Dns);
        }
        let mut connection = self
            .tcp
            .connect(SocketAddr::new(address, url.port_or_default()))
            .await
            .map_err(|e| Error::Transport(e.kind()))?;
        let request = reqwless::request::Request::get(url.path())
            .host(&buffers.request.host)
            .headers(&extra)
            .build();
        request
            .write_header(&mut connection)
            .await
            .map_err(|e| match e {
                reqwless::Error::Network(kind) => Error::Transport(kind),
                _ => Error::Protocol,
            })?;
        connection
            .flush()
            .await
            .map_err(|e| Error::Transport(e.kind()))?;
        let head = response::headers(&mut connection, &mut buffers.response.headers).await?;
        response::body(
            &mut connection,
            &mut buffers.response,
            head,
            source.max_response_bytes() as usize,
            sink,
        )
        .await
    }
}

#[cfg(test)]
mod tests;
