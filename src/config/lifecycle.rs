use super::{CONFIG_FILENAME, Config, ConfigError, ErrorKind, MAX_CONFIG_BYTES};

/// Which source supplied a successfully validated configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// `graphdis.toml` at the MicroSD root.
    MicroSd,
    /// Build-time embedded TOML.
    Compiled,
}

/// Storage result categories; only absence permits fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// No card is present.
    CardMissing,
    /// Card is present but the configuration file does not exist.
    FileMissing,
    /// Card, filesystem, or other I/O error; must remain visible.
    Io,
    /// File exceeds the supplied buffer; must remain visible.
    TooLarge,
}

/// Adapter boundary for a board's MicroSD/filesystem implementation.
pub trait ConfigReader {
    /// Read `filename` into `buffer`, returning bytes copied.
    ///
    /// Report absence accurately; do not convert read failures into FileMissing.
    /// Return TooLarge if more bytes remain than fit, or fill the buffer: the
    /// loader supplies MAX_CONFIG_BYTES + 1 bytes to detect the size boundary.
    fn read(&mut self, filename: &str, buffer: &mut [u8]) -> Result<usize, ReadError>;
}

/// A validated configuration and its selected source.
#[derive(Debug)]
pub struct LoadedConfig {
    /// Complete validated dashboard.
    pub config: Config,
    /// Successful source selection.
    pub origin: ConfigOrigin,
}

/// Load from MicroSD preferentially, falling back only for a missing card/file.
///
/// Scratch must contain at least MAX_CONFIG_BYTES + 1 bytes. It should be static
/// or otherwise reused, rather than allocated in a firmware task's stack frame.
/// Invalid, empty, oversized, or non-UTF-8 present files never trigger fallback.
pub fn load(
    reader: &mut impl ConfigReader,
    scratch: &mut [u8],
    compiled: &str,
) -> Result<LoadedConfig, ConfigError> {
    if scratch.len() < MAX_CONFIG_BYTES + 1 {
        return Err(ConfigError::new(
            ErrorKind::Storage,
            "storage.buffer",
            "configuration read buffer must contain at least 8193 bytes",
        ));
    }
    let input = &mut scratch[..MAX_CONFIG_BYTES + 1];
    match reader.read(CONFIG_FILENAME, input) {
        Ok(size) => {
            if size > MAX_CONFIG_BYTES {
                return Err(ConfigError::limit(
                    ErrorKind::Limit,
                    "storage.file",
                    size,
                    MAX_CONFIG_BYTES,
                ));
            }
            let text = core::str::from_utf8(&input[..size]).map_err(|_| {
                ConfigError::new(
                    ErrorKind::Value,
                    "storage.file",
                    "configuration must be UTF-8",
                )
            })?;
            Ok(LoadedConfig {
                config: Config::parse(text)?,
                origin: ConfigOrigin::MicroSd,
            })
        }
        Err(ReadError::CardMissing | ReadError::FileMissing) => Ok(LoadedConfig {
            config: Config::parse(compiled)?,
            origin: ConfigOrigin::Compiled,
        }),
        Err(ReadError::Io) => Err(ConfigError::new(
            ErrorKind::Storage,
            "storage.file",
            "MicroSD configuration read failed",
        )),
        Err(ReadError::TooLarge) => Err(ConfigError::new(
            ErrorKind::Limit,
            "storage.file",
            "MicroSD configuration exceeds 8192 bytes",
        )),
    }
}

/// In-memory configuration transaction, independent of UI and driver lifecycles.
///
/// Callers must reserve parsing/staging memory and quiesce conflicting storage
/// or request work before reload. This stores configuration only, never a second
/// dashboard history pool. Committing does not itself reconfigure Wi-Fi or the UI.
pub struct ConfigStore {
    active: LoadedConfig,
    generation: u64,
}

/// Successful replacement notification for application task coordination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reload {
    /// New generation; results from older generations must be discarded.
    pub generation: u64,
    /// Origin of the newly committed configuration.
    pub origin: ConfigOrigin,
    /// Successful reload always requires clearing/reusing the history pool.
    pub reset_history: bool,
}

impl ConfigStore {
    /// Install a fully validated initial configuration at generation zero.
    pub fn new(active: LoadedConfig) -> Self {
        Self {
            active,
            generation: 0,
        }
    }
    /// Current immutable configuration and origin.
    pub fn active(&self) -> &LoadedConfig {
        &self.active
    }
    /// Generation used to tag in-flight work.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Stage and validate a replacement, committing only on success.
    /// Any read, syntax, validation, or generation failure preserves active state.
    pub fn reload(
        &mut self,
        reader: &mut impl ConfigReader,
        scratch: &mut [u8],
        compiled: &str,
    ) -> Result<Reload, ConfigError> {
        let generation = self.generation.checked_add(1).ok_or_else(|| {
            ConfigError::new(
                ErrorKind::Limit,
                "reload.generation",
                "configuration generation exhausted",
            )
        })?;
        let replacement = load(reader, scratch, compiled)?;
        self.active = replacement;
        self.generation = generation;
        Ok(Reload {
            generation,
            origin: self.active.origin,
            reset_history: true,
        })
    }
}
