use super::{Error, RequestBuffers};
use crate::config::{MAX_POINTS_PER_SERIES, RequestAuth, Source, SourceKind};
use base64::{Engine, engine::general_purpose::STANDARD};
use core::fmt::Write;

/// A source-specific GET, with timestamps in Unix milliseconds.
/// Query text is deliberately omitted from Debug output.
pub enum Request<'a> {
    /// Fetch the configured direct metrics endpoint, preserving its path/query.
    Metrics,
    /// Evaluate PromQL at server time, or a supplied timestamp.
    Instant {
        /// PromQL text, 1–512 UTF-8 bytes.
        query: &'a str,
        /// Optional Unix milliseconds; omitted means the server's current time.
        time_ms: Option<u64>,
    },
    /// Evaluate PromQL over an explicit history window.
    Range {
        /// PromQL text, 1–512 UTF-8 bytes.
        query: &'a str,
        /// Inclusive start in Unix milliseconds.
        start_ms: u64,
        /// Inclusive end in Unix milliseconds.
        end_ms: u64,
        /// Positive step, in milliseconds, yielding at most 240 samples per series.
        step_ms: u32,
    },
}

impl core::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Metrics => "Metrics",
            Self::Instant { .. } => "Instant([REDACTED])",
            Self::Range { .. } => "Range([REDACTED])",
        })
    }
}

pub(super) fn prepare(
    source: &Source,
    request: Request<'_>,
    buffers: &mut RequestBuffers,
) -> Result<(), Error> {
    buffers.url.clear();
    buffers.host.clear();
    buffers.authorization.clear();
    let url = &mut buffers.url;
    url.push_str(source.url())
        .map_err(|_| Error::RequestTooLarge)?;
    match (source.kind(), request) {
        (SourceKind::Metrics, Request::Metrics) => {}
        (SourceKind::Prometheus, Request::Instant { query, time_ms }) => {
            append_query(url, "/api/v1/query", query)?;
            if let Some(time) = time_ms {
                url.push_str("&time=").map_err(|_| Error::RequestTooLarge)?;
                timestamp(url, time)?;
            }
        }
        (
            SourceKind::Prometheus,
            Request::Range {
                query,
                start_ms,
                end_ms,
                step_ms,
            },
        ) => {
            if step_ms == 0
                || end_ms < start_ms
                || (end_ms - start_ms) / u64::from(step_ms) >= u64::from(MAX_POINTS_PER_SERIES)
            {
                return Err(Error::InvalidRequest);
            }
            append_query(url, "/api/v1/query_range", query)?;
            url.push_str("&start=")
                .map_err(|_| Error::RequestTooLarge)?;
            timestamp(url, start_ms)?;
            url.push_str("&end=").map_err(|_| Error::RequestTooLarge)?;
            timestamp(url, end_ms)?;
            url.push_str("&step=").map_err(|_| Error::RequestTooLarge)?;
            timestamp(url, u64::from(step_ms))?;
        }
        _ => return Err(Error::InvalidRequest),
    }
    let parsed = nourl::Url::parse(url).map_err(|_| Error::InvalidRequest)?;
    buffers
        .host
        .push_str(parsed.host())
        .map_err(|_| Error::RequestTooLarge)?;
    if let Some(port) = parsed.port() {
        write!(buffers.host, ":{port}").map_err(|_| Error::RequestTooLarge)?;
    }
    match source.auth() {
        Some(RequestAuth::Bearer { token }) => {
            buffers
                .authorization
                .push_str("Bearer ")
                .map_err(|_| Error::RequestTooLarge)?;
            buffers
                .authorization
                .push_str(token.expose())
                .map_err(|_| Error::RequestTooLarge)?;
        }
        Some(RequestAuth::Basic { username, password }) => {
            let user = username.expose().as_bytes();
            let password = password.expose().as_bytes();
            let len = user.len() + 1 + password.len();
            let plain = &mut buffers.basic_plain;
            plain[..user.len()].copy_from_slice(user);
            plain[user.len()] = b':';
            plain[user.len() + 1..len].copy_from_slice(password);
            buffers
                .authorization
                .push_str("Basic ")
                .map_err(|_| Error::RequestTooLarge)?;
            let mut encoded = [0; 516];
            let count = STANDARD
                .encode_slice(&plain[..len], &mut encoded)
                .map_err(|_| Error::RequestTooLarge)?;
            let text =
                core::str::from_utf8(&encoded[..count]).map_err(|_| Error::InvalidRequest)?;
            buffers
                .authorization
                .push_str(text)
                .map_err(|_| Error::RequestTooLarge)?;
            plain.fill(0);
        }
        None => {}
    }
    Ok(())
}

fn append_query(url: &mut heapless::String<2304>, path: &str, query: &str) -> Result<(), Error> {
    if query.trim().is_empty()
        || query.len() > 512
        || query
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(Error::InvalidRequest);
    }
    while url.ends_with('/') {
        url.pop();
    }
    url.push_str(path).map_err(|_| Error::RequestTooLarge)?;
    url.push_str("?query=")
        .map_err(|_| Error::RequestTooLarge)?;
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            url.push(char::from(byte))
                .map_err(|_| Error::RequestTooLarge)?;
        } else {
            write!(url, "%{byte:02X}").map_err(|_| Error::RequestTooLarge)?;
        }
    }
    Ok(())
}

fn timestamp(url: &mut heapless::String<2304>, ms: u64) -> Result<(), Error> {
    write!(url, "{}.{:03}", ms / 1000, ms % 1000).map_err(|_| Error::RequestTooLarge)
}
