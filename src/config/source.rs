use super::{ConfigError, ErrorKind, Secret, bounded::Map};
use alloc::{collections::BTreeMap, format, string::String};
use core::fmt;
use serde::Deserialize;

/// Protocol implemented by an HTTP metrics source.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Direct Prometheus exposition endpoint, such as `/metrics`.
    Metrics,
    /// Prometheus server base URL, used with its query API.
    Prometheus,
}

/// HTTP authentication, independently configurable for each source.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestAuth {
    /// HTTP Basic authentication; encode these fields when constructing a request.
    Basic {
        /// Username; redacted from Debug output.
        username: Secret,
        /// Password; redacted from Debug output.
        password: Secret,
    },
    /// A bearer token, without the `Bearer ` prefix.
    Bearer {
        /// Token; redacted from Debug output.
        token: Secret,
    },
}

/// Validated source settings, constructed only by complete configuration validation.
/// URL and headers are deliberately absent from Debug output.
pub struct Source {
    inner: RawSource,
}

// Deserialization input; never exposed as a validated Source.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawSource {
    #[serde(rename = "type")]
    kind: SourceKind,
    url: Secret,
    #[serde(default)]
    headers: Map<Secret, 8>,
    auth: Option<RequestAuth>,
    #[serde(default = "poll_default")]
    poll_interval_secs: u32,
    #[serde(default = "timeout_default")]
    timeout_secs: u32,
    #[serde(default = "response_default")]
    max_response_bytes: u32,
}

fn poll_default() -> u32 {
    15
}
fn timeout_default() -> u32 {
    10
}
fn response_default() -> u32 {
    65_536
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("kind", &self.inner.kind)
            .finish_non_exhaustive()
    }
}

impl Source {
    /// Source protocol.
    pub fn kind(&self) -> SourceKind {
        self.inner.kind
    }
    /// HTTP URL. May contain sensitive query parameters; never log it.
    pub fn url(&self) -> &str {
        self.inner.url.expose()
    }
    /// Validated header names mapped to redacted values.
    pub fn headers(&self) -> &BTreeMap<String, Secret> {
        &self.inner.headers.0
    }
    /// Optional authentication, mutually exclusive with an Authorization header.
    pub fn auth(&self) -> Option<&RequestAuth> {
        self.inner.auth.as_ref()
    }
    /// Polling interval in seconds, 1–3,600.
    pub fn poll_interval_secs(&self) -> u32 {
        self.inner.poll_interval_secs
    }
    /// Overall operation deadline in seconds, 1–120.
    pub fn timeout_secs(&self) -> u32 {
        self.inner.timeout_secs
    }
    /// Maximum bytes consumed from a response, 1–262,144; not a buffer allocation size.
    pub fn max_response_bytes(&self) -> u32 {
        self.inner.max_response_bytes
    }

    fn validate(&self, index: usize) -> Result<(), ConfigError> {
        let field = format!("sources[{index}]");
        let url = self.url();
        let parsed = nourl::Url::parse(url).ok();
        let valid = !url.is_empty()
            && url.len() <= 512
            && url.is_ascii()
            && !url.bytes().any(|b| b <= 0x20 || b == 0x7f)
            && !url.contains(['#', '\\'])
            && parsed.as_ref().is_some_and(|u| {
                u.scheme() == nourl::UrlScheme::HTTP
                    && u.port_or_default() != 0
                    && hostname(u.host())
                    && !u.host().contains(':')
            });
        if !valid {
            return Err(ConfigError::new(
                ErrorKind::Value,
                &format!("{field}.url"),
                "expected an HTTP URL with an IPv4 address or DNS hostname, without credentials or fragment",
            ));
        }
        if self.inner.kind == SourceKind::Prometheus && url.contains('?') {
            return Err(ConfigError::new(
                ErrorKind::Value,
                &format!("{field}.url"),
                "Prometheus base URLs cannot contain query parameters",
            ));
        }
        if !(1..=3600).contains(&self.inner.poll_interval_secs)
            || !(1..=120).contains(&self.inner.timeout_secs)
            || !(1..=262_144).contains(&self.inner.max_response_bytes)
        {
            return Err(ConfigError::new(
                ErrorKind::Value,
                &field,
                "poll interval, timeout, or response limit is out of range",
            ));
        }
        for (header_index, (name, value)) in self.inner.headers.0.iter().enumerate() {
            let path = format!("{field}.headers[{header_index}]");
            if name.is_empty()
                || name.len() > 64
                || !name.bytes().all(header_token)
                || value.expose().len() > 256
                || !value
                    .expose()
                    .bytes()
                    .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
            {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &path,
                    "invalid HTTP header name, value, or length",
                ));
            }
            if [
                "host",
                "content-length",
                "transfer-encoding",
                "connection",
                "upgrade",
                "te",
                "trailer",
                "proxy-connection",
            ]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
            {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &path,
                    "transport-managed headers cannot be overridden",
                ));
            }
            if self
                .inner
                .headers
                .0
                .keys()
                .take(header_index)
                .any(|earlier| earlier.eq_ignore_ascii_case(name))
            {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &path,
                    "duplicate case-insensitive HTTP header",
                ));
            }
            if self.inner.auth.is_some() && name.eq_ignore_ascii_case("authorization") {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &path,
                    "Authorization header conflicts with configured authentication",
                ));
            }
        }
        if let Some(auth) = &self.inner.auth {
            let valid = match auth {
                RequestAuth::Basic { username, password } => {
                    username.expose().len() <= 128
                        && !username.expose().contains(':')
                        && clean(username.expose())
                        && password.expose().len() <= 256
                        && clean(password.expose())
                }
                RequestAuth::Bearer { token } => {
                    let significant = token.expose().trim_end_matches('=');
                    !token.expose().is_empty()
                        && token.expose().len() <= 256
                        && !significant.is_empty()
                        && significant
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b))
                }
            };
            if !valid {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &format!("{field}.auth"),
                    "invalid authentication fields or lengths",
                ));
            }
        }
        Ok(())
    }
}

fn clean(s: &str) -> bool {
    !s.chars().any(char::is_control)
}
fn header_token(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}
fn hostname(host: &str) -> bool {
    let name = host.strip_suffix('.').unwrap_or(host);
    if name.is_empty() || name.len() > 253 {
        return false;
    }
    if name.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return name.parse::<core::net::Ipv4Addr>().is_ok();
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

impl RawSource {
    pub(super) fn resolve(self, index: usize) -> Result<Source, ConfigError> {
        let source = Source { inner: self };
        source.validate(index)?;
        Ok(source)
    }
}
