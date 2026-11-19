//! CYD / Elegoo ESPTouch-2.8: ST7789 on SPI2, shared EN reset.
//! Panel setup matches the working `cyd-test` firmware for ESP32-2432S028R.
//! Write-only SPI leaves GPIO12 (a boot strap) untouched. Touch uses another bus.
use embedded_graphics::{pixelcolor::Rgb565, prelude::*};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    peripherals::{GPIO2, GPIO13, GPIO14, GPIO15, GPIO21, SPI2},
    spi::master::{Config, Spi},
    time::Rate,
};
use mipidsi::{
    Builder, Display, NoResetPin,
    interface::SpiInterface,
    models::ST7789,
    options::{ColorInversion, ColorOrder, Orientation, Rotation},
};
use static_cell::StaticCell;

type Interface = SpiInterface<
    'static,
    ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, NoDelay>,
    Output<'static>,
>;
pub type Lcd = Display<Interface, ST7789, NoResetPin>;
static TRANSFER: StaticCell<[u8; 640]> = StaticCell::new();

#[derive(Clone, Copy, Debug)]
pub enum InitError {
    Spi,
    Display,
    Clear,
    AlreadyStarted,
}

/// Own the backlight alongside the LCD. The backlight stays off until the first
/// complete frame is drawn; initialization failures leave it off.
pub struct BoardDisplay {
    pub lcd: Lcd,
    backlight: Output<'static>,
}
impl BoardDisplay {
    pub fn new(
        spi: SPI2<'static>,
        sck: GPIO14<'static>,
        mosi: GPIO13<'static>,
        cs: GPIO15<'static>,
        dc: GPIO2<'static>,
        backlight: GPIO21<'static>,
    ) -> Result<Self, InitError> {
        let backlight = Output::new(backlight, Level::Low, OutputConfig::default());
        let spi = Spi::new(spi, Config::default().with_frequency(Rate::from_mhz(20)))
            .map_err(|_| InitError::Spi)?
            .with_sck(sck)
            .with_mosi(mosi);
        let cs = Output::new(cs, Level::High, OutputConfig::default());
        let dc = Output::new(dc, Level::Low, OutputConfig::default());
        let spi = ExclusiveDevice::new_no_delay(spi, cs).map_err(|_| InitError::Spi)?;
        let buffer = TRANSFER
            .try_init([0; 640])
            .ok_or(InitError::AlreadyStarted)?;
        let interface = SpiInterface::new(spi, dc, buffer);
        let mut lcd = Builder::new(ST7789, interface)
            .color_order(ColorOrder::Bgr)
            .invert_colors(ColorInversion::Normal)
            .orientation(Orientation::new().rotate(Rotation::Deg90).flip_horizontal())
            .init(&mut Delay::new())
            .map_err(|_| InitError::Display)?;
        lcd.clear(Rgb565::BLACK).map_err(|_| InitError::Clear)?;
        Ok(Self { lcd, backlight })
    }
    pub fn set_backlight(&mut self, on: bool) {
        if on {
            self.backlight.set_high();
        } else {
            self.backlight.set_low();
        }
    }
}
