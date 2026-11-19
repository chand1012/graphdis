//! Persist calibration on MicroSD and mirror it in the dedicated flash partition.
//! Compare generations so stale removable media cannot replace a newer flash copy.
use crate::{
    board::storage::Storage,
    config::ReadError,
    display::touch::{Calibration, newest},
};
use esp_hal::peripherals::FLASH;
use esp_storage::FlashStorage;
const START: u32 = 0x3f0000;
const SECTOR: u32 = 4096;
#[derive(Clone, Copy, Debug)]
pub enum SaveError {
    PartitionMissing,
    Flash,
    Invalid,
    Unavailable,
}
#[derive(Clone, Copy, Debug)]
pub enum SaveLocation {
    MicroSd,
    Flash,
    Both,
}
#[repr(align(4))]
struct Aligned<const N: usize>([u8; N]);
pub struct CalibrationStore {
    flash: FlashStorage<'static>,
    present: bool,
    generation: Option<u32>,
}
impl CalibrationStore {
    pub fn new(flash: FLASH<'static>) -> Self {
        let mut flash = FlashStorage::new(flash);
        let mut present = false;
        let mut overlap = false;
        for i in 0..96 {
            let mut entry = Aligned([0; 32]);
            if flash.read_nor(0x8000 + i * 32, &mut entry.0).is_err() {
                break;
            }
            if entry.0[..2] != [0xaa, 0x50] {
                break;
            }
            let offset = u32::from_le_bytes([entry.0[4], entry.0[5], entry.0[6], entry.0[7]]);
            let size = u32::from_le_bytes([entry.0[8], entry.0[9], entry.0[10], entry.0[11]]);
            if entry.0[2] == 0 && offset.saturating_add(size) > START {
                overlap = true;
            }
            if entry.0[2] == 0x40
                && &entry.0[12..26] == b"graphdis_touch"
                && offset == START
                && size == 0x10000
            {
                present = true;
            }
        }
        Self {
            flash,
            present: present && !overlap,
            generation: None,
        }
    }
    fn records(&mut self) -> Result<[Option<(u32, Calibration)>; 2], SaveError> {
        if !self.present {
            return Err(SaveError::PartitionMissing);
        }
        let mut records = [None, None];
        for (i, r) in records.iter_mut().enumerate() {
            let mut bytes = Aligned([0; 64]);
            self.flash
                .read_nor(START + i as u32 * SECTOR, &mut bytes.0)
                .map_err(|_| SaveError::Flash)?;
            *r = Calibration::decode(&bytes.0);
        }
        Ok(records)
    }
    pub fn load(&mut self, mut sd: Option<&mut Storage>) -> Result<Option<Calibration>, SaveError> {
        let flash = self
            .records()
            .ok()
            .and_then(|r| newest(r).and_then(|i| r[i]));
        let card = sd
            .as_deref_mut()
            .and_then(|s| match s.calibration_records() {
                Ok(r) => newest(r).and_then(|i| r[i]),
                Err(ReadError::CardMissing | ReadError::FileMissing) => None,
                Err(error) => {
                    esp_println::println!("MicroSD calibration read failed: {:?}", error);
                    None
                }
            });
        // Equal generations prefer the card; otherwise choose the newest copy.
        let choices = [flash, card];
        let selected = newest(choices).and_then(|i| choices[i]);
        if let Some((generation, calibration)) = selected {
            self.observe(generation);
            // Adopt existing calibrations automatically. Matching copies do not
            // get rewritten, so normal boots do not wear the card or flash.
            if card.is_none_or(|(g, _)| g != generation)
                && let Some(s) = sd
            {
                match s.save_calibration(calibration, generation) {
                    Ok(()) => esp_println::println!("Calibration copied to MicroSD"),
                    Err(ReadError::CardMissing) => {}
                    Err(error) => {
                        esp_println::println!("MicroSD calibration backup failed: {:?}", error)
                    }
                }
            }
            if flash.is_none_or(|(g, _)| g != generation)
                && let Err(error) = self.save_flash(calibration, generation)
            {
                esp_println::println!("Flash calibration backup unavailable: {:?}", error);
            }
        }
        Ok(selected.map(|(_, c)| c))
    }
    fn observe(&mut self, generation: u32) {
        if self
            .generation
            .is_none_or(|old| generation.wrapping_sub(old) < (1 << 31))
        {
            self.generation = Some(generation);
        }
    }
    pub fn save(
        &mut self,
        c: Calibration,
        mut sd: Option<&mut Storage>,
    ) -> Result<SaveLocation, SaveError> {
        if !c.valid() {
            return Err(SaveError::Invalid);
        }
        let _ = self.load(sd.as_deref_mut());
        let generation = self.generation.unwrap_or(0).wrapping_add(1);
        let flash_saved = match self.save_flash(c, generation) {
            Ok(()) => true,
            Err(error) => {
                esp_println::println!("Flash calibration backup unavailable: {:?}", error);
                false
            }
        };
        let sd_saved = sd.is_some_and(|s| match s.save_calibration(c, generation) {
            Ok(()) => true,
            Err(ReadError::CardMissing) => false,
            Err(error) => {
                esp_println::println!("MicroSD calibration save failed: {:?}", error);
                false
            }
        });
        match (sd_saved, flash_saved) {
            (true, true) => {
                self.observe(generation);
                Ok(SaveLocation::Both)
            }
            (true, false) => {
                self.observe(generation);
                Ok(SaveLocation::MicroSd)
            }
            (false, true) => {
                self.observe(generation);
                Ok(SaveLocation::Flash)
            }
            _ => Err(SaveError::Unavailable),
        }
    }
    fn save_flash(&mut self, c: Calibration, generation: u32) -> Result<(), SaveError> {
        let records = self.records()?;
        let active = newest(records);
        let slot = active.map_or(0, |i| 1 - i);
        let address = START + slot as u32 * SECTOR;
        self.flash
            .erase(address, address + SECTOR)
            .map_err(|_| SaveError::Flash)?;
        let record = Aligned(c.encode(generation));
        self.flash
            .write_nor(address, &record.0)
            .map_err(|_| SaveError::Flash)?;
        let mut verify = Aligned([0; 64]);
        self.flash
            .read_nor(address, &mut verify.0)
            .map_err(|_| SaveError::Flash)?;
        if verify.0 != record.0 {
            return Err(SaveError::Flash);
        }
        Ok(())
    }
}
