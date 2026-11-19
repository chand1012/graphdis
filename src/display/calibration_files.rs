//! Two short-named files hold the same checksummed records as flash storage.
//! Rotate the inactive file, flush, and verify before reporting a saved result.
use super::touch::{Calibration, newest};
pub const FILENAMES: [&str; 2] = ["TOUCH0.CAL", "TOUCH1.CAL"];
pub type Records = [Option<(u32, Calibration)>; 2];
pub trait CalibrationFiles {
    type Error;
    /// Missing or invalid-length files return None. I/O failures remain errors.
    fn read(&mut self, slot: usize) -> Result<Option<[u8; 64]>, Self::Error>;
    /// Implementations must flush data/metadata and close before returning.
    fn write(&mut self, slot: usize, record: &[u8; 64]) -> Result<(), Self::Error>;
}
#[derive(Debug)]
pub enum FileError<E> {
    Storage(E),
    Invalid,
    Verify,
}
pub fn load<F: CalibrationFiles>(files: &mut F) -> Result<Records, F::Error> {
    let mut records = [None, None];
    let mut error = None;
    for (i, record) in records.iter_mut().enumerate() {
        match files.read(i) {
            Ok(bytes) => *record = bytes.as_ref().and_then(Calibration::decode),
            Err(failure) => {
                if error.is_none() {
                    error = Some(failure);
                }
            }
        }
    }
    if records.iter().any(Option::is_some) {
        return Ok(records);
    }
    if let Some(error) = error {
        Err(error)
    } else {
        Ok(records)
    }
}
pub fn save<F: CalibrationFiles>(
    files: &mut F,
    calibration: Calibration,
    generation: u32,
) -> Result<(), FileError<F::Error>> {
    if !calibration.valid() {
        return Err(FileError::Invalid);
    }
    let records = load(files).map_err(FileError::Storage)?;
    let target = newest(records).map_or(0, |i| 1 - i);
    let encoded = calibration.encode(generation);
    files.write(target, &encoded).map_err(FileError::Storage)?;
    if files.read(target).map_err(FileError::Storage)? != Some(encoded) {
        return Err(FileError::Verify);
    }
    Ok(())
}
