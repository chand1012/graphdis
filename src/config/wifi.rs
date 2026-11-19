use super::{ConfigError, ErrorKind};
use alloc::string::String;
use core::fmt;
use serde::Deserialize;

/// Secret string whose Debug representation never reveals its contents.
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Explicitly access the secret for driver configuration or request construction.
    /// Never include this result in diagnostics.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Supported station authentication; WPA2 never implicitly downgrades to open.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Authentication {
    /// WPA2-Personal with a passphrase or hexadecimal key.
    #[default]
    Wpa2Personal,
    /// Intentionally open Wi-Fi, requiring an empty password.
    Open,
}

/// Wi-Fi settings validated as part of the complete dashboard configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WifiConfig {
    ssid: Secret,
    #[serde(default)]
    password: Secret,
    #[serde(default)]
    authentication: Authentication,
}

impl WifiConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        let ssid = self.ssid.expose();
        if ssid.is_empty() || ssid.len() > 32 || ssid.contains('\0') {
            return Err(ConfigError::new(
                ErrorKind::Value,
                "wifi.ssid",
                "expected 1–32 UTF-8 bytes without NUL",
            ));
        }
        let password = self.password.expose().as_bytes();
        let valid = match self.authentication {
            Authentication::Open => password.is_empty(),
            Authentication::Wpa2Personal => {
                ((8..=63).contains(&password.len())
                    && password.iter().all(|b| (0x20..=0x7e).contains(b)))
                    || (password.len() == 64 && password.iter().all(u8::is_ascii_hexdigit))
            }
        };
        if !valid {
            return Err(ConfigError::new(
                ErrorKind::Value,
                "wifi.password",
                "password is invalid for the selected authentication",
            ));
        }
        Ok(())
    }

    /// Network name, validated as 1–32 bytes without NUL.
    pub fn ssid(&self) -> &str {
        self.ssid.expose()
    }
    /// Authentication policy.
    pub fn authentication(&self) -> Authentication {
        self.authentication
    }
    #[cfg(feature = "firmware")]
    pub(crate) fn password(&self) -> &str {
        self.password.expose()
    }
}
