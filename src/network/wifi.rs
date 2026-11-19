use embassy_net::Stack;
use embassy_time::{Duration, Timer, with_timeout};
use esp_radio::wifi::{
    AuthenticationMethodConfig, ConnectionError, WifiController, sta::StationConfig,
};

use super::{NetworkStatus, RetryReason, STATUS, StartError};
use crate::{
    config::{Authentication, WifiConfig},
    retry::Backoff,
};

pub(super) fn station_config(config: &WifiConfig) -> Result<StationConfig, StartError> {
    if config.ssid() == "YOUR_WIFI_SSID" || config.password() == "YOUR_WIFI_PASSWORD" {
        esp_println::println!(
            "Wi-Fi configuration still contains placeholders; edit [wifi] in the selected TOML"
        );
    }
    let authentication = match config.authentication() {
        Authentication::Open => AuthenticationMethodConfig::Open,
        Authentication::Wpa2Personal => AuthenticationMethodConfig::Wpa2Personal(
            config
                .password()
                .try_into()
                .map_err(|_| StartError::RadioConfiguration)?,
        ),
    };
    Ok(StationConfig::default()
        .with_ssid(
            config
                .ssid()
                .try_into()
                .map_err(|_| StartError::RadioConfiguration)?,
        )
        .with_authentication(authentication))
}

#[embassy_executor::task]
pub(super) async fn connection_task(
    mut controller: WifiController<'static>,
    stack: Stack<'static>,
) -> ! {
    let status = STATUS.sender();
    let mut backoff = Backoff::new();
    loop {
        status.send(NetworkStatus::Connecting);
        let reason = match with_timeout(Duration::from_secs(20), controller.connect_async()).await {
            Ok(Ok(_)) => {
                status.send(NetworkStatus::WaitingForAddress);
                // Link loss during DHCP is also bounded by this deadline.
                match with_timeout(Duration::from_secs(30), stack.wait_config_up()).await {
                    Ok(()) if controller.is_connected() && stack.is_link_up() => {
                        if let Some(config) = stack.config_v4() {
                            backoff.reset();
                            let mut address = config.address.address().octets();
                            status.send(NetworkStatus::Online { address });
                            // Poll the link/lease as well as driver state. This avoids
                            // hanging on a missed disconnect event and notices lease loss.
                            while controller.is_connected()
                                && stack.is_link_up()
                                && stack.is_config_up()
                            {
                                if let Some(config) = stack.config_v4() {
                                    let renewed = config.address.address().octets();
                                    if renewed != address {
                                        address = renewed;
                                        status.send(NetworkStatus::Online { address });
                                    }
                                }
                                Timer::after(Duration::from_millis(250)).await;
                            }
                            RetryReason::Disconnected
                        } else {
                            RetryReason::AddressTimeout
                        }
                    }
                    _ => RetryReason::AddressTimeout,
                }
            }
            Ok(Err(error)) => {
                // DisconnectedInfo also contains SSID/BSSID. Log only the
                // reason, never the full connection error or configuration.
                match error {
                    ConnectionError::Failed(info) => {
                        esp_println::println!("Wi-Fi connection failed: {:?}", info.reason);
                    }
                    ConnectionError::WifiError(error) => {
                        esp_println::println!("Wi-Fi driver error: {:?}", error);
                    }
                    _ => {
                        esp_println::println!("Wi-Fi connection failed: unclassified driver error")
                    }
                }
                RetryReason::ConnectionFailed
            }
            Err(_) => RetryReason::ConnectionTimeout,
        };
        // A canceled connect future can leave a driver-side attempt running.
        // Explicitly abort it before retrying; teardown must also have a deadline.
        let _ = with_timeout(Duration::from_secs(5), controller.disconnect_async()).await;
        let after_secs = backoff.next_secs();
        status.send(NetworkStatus::Retrying { reason, after_secs });
        Timer::after(Duration::from_secs(u64::from(after_secs))).await;
    }
}
