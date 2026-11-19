use crate::{
    config::{ConfigReader, ReadError},
    display::{
        calibration_files::{self, CalibrationFiles, FILENAMES, Records},
        touch::Calibration,
    },
};
use embedded_hal::spi::SpiBus;
use embedded_hal_bus::spi::ExclusiveDevice;
use embedded_sdmmc::{Directory, Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
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
/// FAT16/FAT32, first MBR partition. Dashboard TOML stays read-only; only
/// calibration files are created/updated. No card-detect contact is exposed
/// by this board. An unresponsive idle command is treated as an absent card.
pub struct Storage {
    card: Option<Card>,
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
        Ok(Self {
            card: Some(SdCard::new(device, Delay::new())),
        })
    }
}
impl Storage {
    fn mounted<R>(
        &mut self,
        operation: impl FnOnce(&VolumeManager<Card, FixedClock>) -> Result<R, ReadError>,
    ) -> Result<R, ReadError> {
        let card = self.card.take().ok_or(ReadError::Io)?;
        card.mark_card_uninit();
        let initialization = card.num_bytes();
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
            operation(&manager)
        })();
        let (card, _) = manager.free();
        self.card = Some(card);
        result
    }
    pub fn calibration_records(&mut self) -> Result<Records, ReadError> {
        self.mounted(|manager| {
            let volume = manager
                .open_volume(VolumeIdx(0))
                .map_err(|_| ReadError::Io)?;
            let root = volume.open_root_dir().map_err(|_| ReadError::Io)?;
            calibration_files::load(&mut SdFiles { root })
        })
    }
    pub fn save_calibration(
        &mut self,
        calibration: Calibration,
        generation: u32,
    ) -> Result<(), ReadError> {
        self.mounted(|manager| {
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
        self.mounted(|manager| {
            let volume = manager
                .open_volume(VolumeIdx(0))
                .map_err(|_| ReadError::Io)?;
            let root = volume.open_root_dir().map_err(|_| ReadError::Io)?;
            // .toml has four extension characters, so use the long-filename API.
            let file = root
                .open_long_name_file_in_dir(filename, Mode::ReadOnly)
                .map_err(|e| match e {
                    embedded_sdmmc::Error::NotFound => ReadError::FileMissing,
                    _ => ReadError::Io,
                })?;
            let mut len = 0;
            while len < buffer.len() {
                let read = file.read(&mut buffer[len..]).map_err(|_| ReadError::Io)?;
                if read == 0 {
                    break;
                }
                len += read;
            }
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
