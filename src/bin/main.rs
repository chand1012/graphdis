#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use graphdis::{
    board::{storage::Storage, touch::Touch},
    display::board::BoardDisplay,
    timing,
    timing::Span,
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    esp_println::println!("Unrecoverable firmware fault; restarting");
    esp_hal::system::software_reset()
}

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 2.0.0
    // generator parameters: -o esp32 -o esp32-wroom-32 -o alloc -o unstable-hal -o embassy -o wifi -o ci -o agents -o vscode -o zed

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    // The monotonic counter starts inside esp_hal::init; bootloader time is excluded.
    timing!("boot: esp_hal::init done, CPU clock max");

    // The following pins are used to bootstrap the chip. They are available
    // for use, but check the datasheet of the module for more information on them.
    // - GPIO0
    // - GPIO2
    // - GPIO5
    // - GPIO12
    // - GPIO15
    // These GPIO pins are in use by some feature of the module and should not be used.
    let _gpio6 = peripherals.GPIO6;
    let _gpio7 = peripherals.GPIO7;
    let _gpio8 = peripherals.GPIO8;
    let _gpio9 = peripherals.GPIO9;
    let _gpio10 = peripherals.GPIO10;
    let _gpio11 = peripherals.GPIO11;
    let _gpio16 = peripherals.GPIO16;
    let _gpio20 = peripherals.GPIO20;

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 98768);
    // Keep extra headroom for validation while radio and active metadata live.
    esp_alloc::heap_allocator!(size: 32768);
    timing!("boot: heap ready, {} bytes free", esp_alloc::HEAP.free());

    let span = Span::start("boot: esp-rtos start");
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);
    span.done();

    let span = Span::start("boot: display init (SPI, ST7789, clear)");
    let display = BoardDisplay::new(
        peripherals.SPI2,
        peripherals.GPIO14,
        peripherals.GPIO13,
        peripherals.GPIO15,
        peripherals.GPIO2,
        peripherals.GPIO21,
    );
    span.done_with(format_args!("ok={}", display.is_ok()));
    let display = match display {
        Ok(display) => display,
        Err(error) => {
            esp_println::println!("Display initialization failed: {:?}", error);
            loop {
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    };
    let span = Span::start("boot: MicroSD SPI init");
    let storage = Storage::new(
        peripherals.SPI3,
        peripherals.GPIO18,
        peripherals.GPIO19,
        peripherals.GPIO23,
        peripherals.GPIO5,
    );
    span.done_with(format_args!("ok={}", storage.is_ok()));
    let span = Span::quiet("boot: touch GPIO init");
    let touch = Touch::new(
        peripherals.GPIO25,
        peripherals.GPIO32,
        peripherals.GPIO39,
        peripherals.GPIO33,
        peripherals.GPIO36,
    );
    span.done();
    timing!("boot: hardware ready; entering app::run");
    graphdis::app::run(
        spawner,
        display,
        storage,
        touch,
        peripherals.WIFI,
        peripherals.FLASH,
    )
    .await
}
