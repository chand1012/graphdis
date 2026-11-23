use super::{Network, TcpResources, TcpTransport};
use crate::timing::Span;
use crate::{
    config::Source,
    sources::http::{BodySink, Client, EmbassyDeadline, Error, HttpBuffers, Request, Summary},
};
use core::net::{IpAddr, SocketAddr};
use embassy_net::dns::DnsSocket;
use embedded_nal_async::{AddrType, Dns, TcpConnect};

/// Firmware adapter borrowing the collection task's fixed TCP pool.
/// Each concurrent fetch also needs its own HttpBuffers and transactional sink.
pub struct HttpClient<'a> {
    tcp: TcpTransport<'a>,
    dns: DnsSocket<'static>,
}

impl Network {
    /// Create an HTTP adapter over this stack; at most two application connections
    /// may use the shared pool. Initialize pool and request buffers in place.
    pub fn http_client<'a>(&self, resources: &'a TcpResources) -> HttpClient<'a> {
        HttpClient {
            tcp: self.tcp_client(resources),
            dns: self.dns_client(),
        }
    }
}

impl HttpClient<'_> {
    /// Fetch one source using its authentication, headers, byte limit, and deadline.
    pub async fn fetch<S: BodySink>(
        &self,
        source: &Source,
        request: Request<'_>,
        buffers: &mut HttpBuffers,
        sink: &mut S,
    ) -> Result<Summary, Error> {
        Client::new(&Timed(&self.tcp), &Timed(&self.dns), &EmbassyDeadline)
            .fetch(source, request, buffers, sink)
            .await
    }
}

/// Logs DNS lookup and TCP connect durations without logging hosts or addresses.
struct Timed<'a, T>(&'a T);

impl<T: Dns> Dns for Timed<'_, T> {
    type Error = T::Error;
    async fn get_host_by_name(&self, host: &str, kind: AddrType) -> Result<IpAddr, Self::Error> {
        let span = Span::quiet("http: DNS lookup");
        let result = self.0.get_host_by_name(host, kind).await;
        span.done_with(format_args!("ok={}", result.is_ok()));
        result
    }
    async fn get_host_by_address(
        &self,
        address: IpAddr,
        result: &mut [u8],
    ) -> Result<usize, Self::Error> {
        self.0.get_host_by_address(address, result).await
    }
}

impl<T: TcpConnect> TcpConnect for Timed<'_, T> {
    type Error = T::Error;
    type Connection<'a>
        = T::Connection<'a>
    where
        Self: 'a;
    async fn connect<'a>(
        &'a self,
        remote: SocketAddr,
    ) -> Result<Self::Connection<'a>, Self::Error> {
        let span = Span::quiet("http: TCP connect");
        let result = self.0.connect(remote).await;
        span.done_with(format_args!("ok={}", result.is_ok()));
        result
    }
}
