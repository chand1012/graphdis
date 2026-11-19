use embedded_hal::delay::DelayNs;
use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    peripherals::{GPIO25, GPIO32, GPIO33, GPIO36, GPIO39},
};
/// XPT2046 has a separate SPI wiring set. Software SPI avoids sharing either the
/// LCD or SD controller and runs at a conservative clock below 500 kHz.
pub struct Touch {
    clock: Output<'static>,
    mosi: Output<'static>,
    miso: Input<'static>,
    cs: Output<'static>,
    irq: Input<'static>,
    active: bool,
}
impl Touch {
    pub fn new(
        clock: GPIO25<'static>,
        mosi: GPIO32<'static>,
        miso: GPIO39<'static>,
        cs: GPIO33<'static>,
        irq: GPIO36<'static>,
    ) -> Self {
        Self {
            clock: Output::new(clock, Level::Low, OutputConfig::default()),
            mosi: Output::new(mosi, Level::Low, OutputConfig::default()),
            miso: Input::new(miso, InputConfig::default()),
            cs: Output::new(cs, Level::High, OutputConfig::default()),
            irq: Input::new(irq, InputConfig::default()),
            active: false,
        }
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        let mut input = 0;
        let mut delay = Delay::new();
        for bit in (0..8).rev() {
            if byte & (1 << bit) != 0 {
                self.mosi.set_high();
            } else {
                self.mosi.set_low();
            }
            delay.delay_us(1);
            self.clock.set_high();
            delay.delay_us(1);
            input = (input << 1) | u8::from(self.miso.is_high());
            self.clock.set_low();
        }
        input
    }
    fn read(&mut self, command: u8) -> u16 {
        self.cs.set_low();
        Delay::new().delay_us(2);
        self.transfer(command);
        let high = self.transfer(0);
        let low = self.transfer(0);
        self.cs.set_high();
        ((u16::from(high) << 8) | u16::from(low)) >> 3 & 0xfff
    }
    pub fn sample(&mut self) -> Option<(u16, u16)> {
        if self.irq.is_high() && !self.active {
            return None;
        }
        // Confirm contact by pressure. PENIRQ changes state during conversions,
        // so sampling it immediately after an ADC read can fabricate releases.
        let z1 = self.read(0xb0);
        let z2 = self.read(0xc0);
        let pressure = u32::from(z1) + 4095 - u32::from(z2);
        if z1 < 20 || pressure < 80 {
            self.active = false;
            return None;
        }
        let mut x = [0; 5];
        let mut y = [0; 5];
        for i in 0..5 {
            x[i] = self.read(0xd0);
            y[i] = self.read(0x90);
        }
        x.sort_unstable();
        y.sort_unstable();
        if !(50..=4045).contains(&x[2]) || !(50..=4045).contains(&y[2]) {
            self.active = false;
            None
        } else {
            self.active = true;
            Some((x[2], y[2]))
        }
    }
}
