use crate::{
    config::{ConfigReader, ReadError},
    display::{
        calibration_files::{self, CalibrationFiles, FILENAMES, Records},
        touch::Calibration,
    },
    timing::{Ms, Span},
};
use embedded_hal::spi::SpiBus;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::{
    Directory, Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager,
    sdcard::spi::AcquireOpts,
};
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    peripherals::{GPIO5, GPIO18, GPIO19, GPIO23, SPI3},
    spi::master::{Config, Spi},
    time::Rate,
};

type Card = SdCard<ExclusiveDevice<Spi<'static, esp_hal::Blocking>, Output<'static>, Delay>, Delay>;
struct FixedClock;
impl TimeSource for FixedClock {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 10,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}
/// CMD0 attempts before a silent card is reported absent. Each attempt polls up
/// to 10,000 response bytes at 400 kHz, about 0.53 s on this board; the driver
/// default of 50 made an absent card cost 26 s per access. Responsive cards
/// normally answer the first or second CMD0.
const ACQUIRE_RETRIES: u32 = 2;

/// FAT16/FAT32, first MBR partition. Dashboard TOML stays read-only; only
/// calibration files are created/updated. No card-detect contact is exposed
/// by this board. An unresponsive idle command is treated as an absent card.
pub struct Storage {
    card: Option<Card>,
    /// Set when the card did not answer CMD0. Later accesses report it absent
    /// immediately until [`Storage::reprobe`], since probing is slow and the
    /// card must not be inserted while powered.
    absent: bool,
}
impl Storage {
    pub fn new(
        spi: SPI3<'static>,
        sck: GPIO18<'static>,
        miso: GPIO19<'static>,
        mosi: GPIO23<'static>,
        cs: GPIO5<'static>,
    ) -> Result<Self, ReadError> {
        let cs = Output::new(cs, Level::High, OutputConfig::default());
        let mut spi = Spi::new(spi, Config::default().with_frequency(Rate::from_khz(400)))
            .map_err(|_| ReadError::Io)?
            .with_sck(sck)
            .with_mosi(mosi)
            .with_miso(miso);
        // SD requires at least 74 idle clocks with CS high before CMD0.
        spi.write(&[0xff; 10]).map_err(|_| ReadError::Io)?;
        spi.flush().map_err(|_| ReadError::Io)?;
        let device = ExclusiveDevice::new(spi, cs, Delay::new()).map_err(|_| ReadError::Io)?;
        let options = AcquireOpts {
            use_crc: true,
            acquire_retries: ACQUIRE_RETRIES,
        };
        Ok(Self {
            card: Some(SdCard::new_with_options(device, Delay::new(), options)),
            absent: false,
        })
    }
    /// Probe the card again on the next access, for explicit user actions.
    pub fn reprobe(&mut self) {
        self.absent = false;
    }
}
impl Storage {
    /// Each call re-initializes the card at 400 kHz, then mounts the volume.
    fn mounted<R>(
        &mut self,
        label: &'static str,
        operation: impl FnOnce(&VolumeManager<Card, FixedClock>) -> Result<R, ReadError>,
    ) -> Result<R, ReadError> {
        if self.absent {
            crate::timing!("{}: skipped, card absent at earlier probe", label);
            return Err(ReadError::CardMissing);
        }
        let card = self.card.take().ok_or(ReadError::Io)?;
        let total = Span::start(label);
        card.mark_card_uninit();
        let span = Span::quiet("sd: card init (CMD0..ACMD41) @400kHz");
        let initialization = card.num_bytes();
        span.done_with(format_args!("ok={}", initialization.is_ok()));
        let manager = VolumeManager::new(card, FixedClock);
        let result = (|| {
            if let Err(error) = initialization {
                return Err(match error {
                    // The driver retries CMD0 internally and ultimately reports
                    // CardNotFound, rather than returning the original timeout.
                    embedded_sdmmc::SdCardError::CardNotFound => ReadError::CardMissing,
                    // CMD0 is command number zero. Other timeouts indicate a card
                    // that responded and then failed, so they must remain visible.
                    embedded_sdmmc::SdCardError::TimeoutCommand(command) if command as u8 == 0 => {
                        ReadError::CardMissing
                    }
                    _ => {
                        esp_println::println!("MicroSD initialization error: {:?}", error);
                        ReadError::Io
                    }
                });
            }
            let span = Span::quiet("sd: filesystem operation");
            let result = operation(&manager);
            span.done_with(format_args!("ok={}", result.is_ok()));
            result
        })();
        let (card, _) = manager.free();
        self.card = Some(card);
        // Only an unanswered CMD0 maps to CardMissing; cache that one result.
        self.absent = initialization.is_err() && matches!(result, Err(ReadError::CardMissing));
        total.done_with(format_args!("ok={}", result.is_ok()));
        result
    }
    pub fn calibration_records(&mut self) -> Result<Records, ReadError> {
        self.mounted("sd: read calibration records", |manager| {
            let span = Span::quiet("sd: open volume + root");
            let volume = manager
                .open_volume(VolumeIdx(0))
                .map_err(|_| ReadError::Io)?;
            let root = volume.open_root_dir().map_err(|_| ReadError::Io)?;
            span.done();
            let span = Span::quiet("sd: read calibration files");
            let records = calibration_files::load(&mut SdFiles { root });
            span.done();
            records
        })
    }
    pub fn save_calibration(
        &mut self,
        calibration: Calibration,
        generation: u32,
    ) -> Result<(), ReadError> {
        self.mounted("sd: save calibration record", |manager| {
            let volume = manager
                .open_volume(VolumeIdx(0))
                .map_err(|_| ReadError::Io)?;
            let root = volume.open_root_dir().map_err(|_| ReadError::Io)?;
            calibration_files::save(&mut SdFiles { root }, calibration, generation)
                .map_err(|_| ReadError::Io)
        })
    }
}
impl ConfigReader for Storage {
    fn read(&mut self, filename: &str, buffer: &mut [u8]) -> Result<usize, ReadError> {
        self.mounted("sd: read config file", |manager| {
            let span = Span::quiet("sd: open volume + root");
            let volume = manager
                .open_volume(VolumeIdx(0))
                .map_err(|_| ReadError::Io)?;
            let root = volume.open_root_dir().map_err(|_| ReadError::Io)?;
            span.done();
            let span = Span::quiet("sd: open config (long-name lookup)");
            // .toml has four extension characters, so use the long-filename API.
            let file = root
                .open_long_name_file_in_dir(filename, Mode::ReadOnly)
                .map_err(|e| match e {
                    embedded_sdmmc::Error::NotFound => ReadError::FileMissing,
                    _ => ReadError::Io,
                })?;
            span.done();
            let span = Span::quiet("sd: read config bytes");
            let mut len = 0;
            while len < buffer.len() {
                let read = file.read(&mut buffer[len..]).map_err(|_| ReadError::Io)?;
                if read == 0 {
                    break;
                }
                len += read;
            }
            let elapsed = span.done_with(format_args!("{} bytes", len));
            let kib_per_s = (len as u64 * 1_000_000 / 1024)
                .checked_div(elapsed.as_micros())
                .unwrap_or(0);
            crate::timing!(
                "sd: config throughput {} KiB/s ({} for {} bytes)",
                kib_per_s,
                Ms(elapsed),
                len
            );
            if len == buffer.len() {
                return Err(ReadError::TooLarge);
            }
            Ok(len)
        })
    }
}
struct SdFiles<'a> {
    root: Directory<'a, Card, FixedClock, 4, 4, 1>,
}
impl CalibrationFiles for SdFiles<'_> {
    type Error = ReadError;
    fn read(&mut self, slot: usize) -> Result<Option<[u8; 64]>, ReadError> {
        let file = match self.root.open_file_in_dir(FILENAMES[slot], Mode::ReadOnly) {
            Ok(file) => file,
            Err(embedded_sdmmc::Error::NotFound) => return Ok(None),
            Err(_) => return Err(ReadError::Io),
        };
        if file.length() != 64 {
            return Ok(None);
        }
        let mut record = [0; 64];
        let mut used = 0;
        while used < record.len() {
            let size = file.read(&mut record[used..]).map_err(|_| ReadError::Io)?;
            if size == 0 {
                return Ok(None);
            }
            used += size;
        }
        file.close().map_err(|_| ReadError::Io)?;
        Ok(Some(record))
    }
    fn write(&mut self, slot: usize, record: &[u8; 64]) -> Result<(), ReadError> {
        let file = self
            .root
            .open_file_in_dir(FILENAMES[slot], Mode::ReadWriteCreateOrTruncate)
            .map_err(|_| ReadError::Io)?;
        file.write(record).map_err(|_| ReadError::Io)?;
        file.flush().map_err(|_| ReadError::Io)?;
        file.close().map_err(|_| ReadError::Io)
    }
}
