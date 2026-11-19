use super::{Network, TcpResources, TcpTransport};
use crate::{
    config::Source,
    sources::http::{BodySink, Client, EmbassyDeadline, Error, HttpBuffers, Request, Summary},
};
use embassy_net::dns::DnsSocket;

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
        Client::new(&self.tcp, &self.dns, &EmbassyDeadline)
            .fetch(source, request, buffers, sink)
            .await
    }
}
