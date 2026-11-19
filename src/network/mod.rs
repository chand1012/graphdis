//! ESP32 station networking: owned Wi-Fi and IP runner tasks, DHCP, DNS, and TCP.

mod http;
mod wifi;

pub use http::HttpClient;

use embassy_executor::Spawner;
use embassy_net::{
    Runner, Stack, StackResources,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    watch::{Receiver, Watch},
};
use embassy_time::{Duration, with_timeout};
use esp_hal::{peripherals::WIFI, rng::Rng};
use esp_radio::wifi::{Interface, WifiController};
use static_cell::StaticCell;

use crate::config::WifiConfig;

/// Four sockets: one DHCP, one DNS, and at most two application TCP sockets.
pub const SOCKET_COUNT: usize = 4;
/// Maximum TCP connections managed by an application transport pool.
pub const TCP_CONNECTIONS: usize = 2;
/// Per-connection TCP transmit buffer size, in bytes.
pub const TCP_TX_BYTES: usize = 1024;
/// Per-connection TCP receive buffer size, in bytes.
pub const TCP_RX_BYTES: usize = 2048;
/// Application-owned TCP pool: 6 KiB of packet buffers, plus metadata.
pub type TcpResources = TcpClientState<TCP_CONNECTIONS, TCP_TX_BYTES, TCP_RX_BYTES>;
/// TCP transport compatible with the HTTP client's embedded networking traits.
pub type TcpTransport<'a> = TcpClient<'a, TCP_CONNECTIONS, TCP_TX_BYTES, TCP_RX_BYTES>;
/// A status subscriber; at most four may exist at once.
pub type StatusReceiver = Receiver<'static, CriticalSectionRawMutex, NetworkStatus, 4>;

static RESOURCES: StaticCell<StackResources<SOCKET_COUNT>> = StaticCell::new();
static STATUS: Watch<CriticalSectionRawMutex, NetworkStatus, 4> =
    Watch::new_with(NetworkStatus::Idle);

/// Secret-free snapshot of connection progress, published by the Wi-Fi owner task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStatus {
    /// No networking task has been started.
    Idle,
    /// Associating and authenticating with the configured access point.
    Connecting,
    /// Wi-Fi is connected, but DHCP has not supplied an IPv4 address yet.
    WaitingForAddress,
    /// The link is connected and DHCP supplied the indicated IPv4 address.
    Online {
        /// Local IPv4 address, in network-byte order.
        address: [u8; 4],
    },
    /// Connection will be retried after a bounded delay.
    Retrying {
        /// Category of failure; never includes credentials or source responses.
        reason: RetryReason,
        /// Delay before the next attempt, in seconds.
        after_secs: u32,
    },
}

/// Recoverable failure categories used by connection diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryReason {
    /// Association or authentication failed.
    ConnectionFailed,
    /// Connection did not complete within 20 seconds.
    ConnectionTimeout,
    /// DHCP did not complete within 30 seconds.
    AddressTimeout,
    /// Link or address was lost after a successful connection.
    Disconnected,
}

/// Startup failures returned to the application instead of panicking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    /// Validated settings could not be applied by the driver.
    RadioConfiguration,
    /// Wi-Fi initialization failed.
    RadioInitialization,
    /// A station interface or the one-shot stack resources were already taken.
    AlreadyStarted,
    /// A statically allocated task slot was unavailable.
    TaskUnavailable,
}

/// An operation could not obtain a usable link and address within its deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadyTimeout;

/// Copyable handle; the background tasks own the controller, driver, and runner.
#[derive(Clone, Copy)]
pub struct Network {
    stack: Stack<'static>,
}

impl Network {
    /// Start station mode once, after heap and esp-rtos initialization.
    ///
    /// The driver is configured before tasks are spawned. Stack resources are
    /// static, never placed in a large local stack frame. A task startup failure
    /// is terminal for this boot; do not repeatedly call this one-shot function.
    pub fn start(
        spawner: Spawner,
        peripheral: WIFI<'static>,
        config: &WifiConfig,
    ) -> Result<Self, StartError> {
        let station = wifi::station_config(config)?;
        let controller = WifiController::new(
            peripheral,
            esp_radio::wifi::ControllerConfig::default()
                .with_initial_config(esp_radio::wifi::Config::Station(station)),
        )
        .map_err(|_| StartError::RadioInitialization)?;
        let interface = Interface::try_station().ok_or(StartError::AlreadyStarted)?;
        let resources = RESOURCES
            .try_init_with(StackResources::new)
            .ok_or(StartError::AlreadyStarted)?;
        let rng = Rng::new();
        let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
        let (stack, runner) = embassy_net::new(
            interface,
            embassy_net::Config::dhcpv4(Default::default()),
            resources,
            seed,
        );
        spawner.spawn(runner_task(runner).map_err(|_| StartError::TaskUnavailable)?);
        spawner.spawn(
            wifi::connection_task(controller, stack).map_err(|_| StartError::TaskUnavailable)?,
        );
        Ok(Self { stack })
    }

    /// Underlying IPv4 stack for bounded application protocols.
    ///
    /// The stack has two application socket slots. Pool resources, HTTP response
    /// buffers, and operation deadlines remain the caller's responsibility.
    pub fn stack(&self) -> Stack<'static> {
        self.stack
    }

    /// Current link/address readiness; unlike status snapshots, this is live state.
    pub fn is_ready(&self) -> bool {
        self.stack.is_link_up() && self.stack.is_config_up()
    }

    /// Wait for both Wi-Fi link and IPv4 configuration, bounded by `timeout`.
    ///
    /// Connectivity may change immediately afterwards; callers must still handle
    /// DNS/TCP/HTTP failures and apply their own operation deadlines.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), ReadyTimeout> {
        with_timeout(timeout, async {
            loop {
                self.stack.wait_link_up().await;
                self.stack.wait_config_up().await;
                if self.is_ready() {
                    break;
                }
            }
        })
        .await
        .map_err(|_| ReadyTimeout)
    }

    /// Subscribe to latest status, or return `None` when four subscribers exist.
    ///
    /// This is a single-snapshot watch channel, not an event log: new status
    /// overwrites old status, so a slow consumer cannot stall reconnection.
    pub fn subscribe(&self) -> Option<StatusReceiver> {
        STATUS.receiver()
    }

    /// Create a DNS adapter using servers supplied by DHCP; no packet buffer allocation.
    ///
    /// Bound each query with an application deadline when used by a higher-level client.
    pub fn dns_client(&self) -> DnsSocket<'static> {
        DnsSocket::new(self.stack)
    }

    /// Create a TCP client borrowing the collection task's fixed-capacity pool.
    ///
    /// `resources` consumes about 6 KiB plus metadata and should be initialized
    /// statically or in place. Sockets have a 15-second inactivity timeout; use
    /// a separate overall deadline for DNS, connect, and HTTP operations.
    pub fn tcp_client<'a>(&self, resources: &'a TcpResources) -> TcpTransport<'a> {
        let mut client = TcpClient::new(self.stack, resources);
        client.set_timeout(Some(Duration::from_secs(15)));
        client
    }
}

#[embassy_executor::task]
async fn runner_task(mut runner: Runner<'static, Interface>) -> ! {
    runner.run().await
}
