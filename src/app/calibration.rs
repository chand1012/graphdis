use crate::{
    board::{
        calibration::{CalibrationStore, SaveLocation},
        storage::Storage,
        touch::Touch,
    },
    config::Palette,
    display::{board::BoardDisplay, controls, touch::Calibration},
};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::prelude::Point;
pub(super) async fn wait_release(touch: &mut Touch, timeout: Duration) -> bool {
    let end = Instant::now() + timeout;
    let mut count = 0;
    while Instant::now() < end {
        Timer::after(Duration::from_millis(25)).await;
        if touch.sample().is_none() {
            count += 1;
            if count >= 3 {
                return true;
            }
        } else {
            count = 0;
        }
    }
    false
}
/// Explicit, numbered capture with stable hold feedback and a fourth independent
/// check point. A failed/cancelled calibration never replaces the previous one.
pub(super) async fn calibrate(
    lcd: &mut BoardDisplay,
    touch: &mut Touch,
    palette: &Palette,
    store: &mut CalibrationStore,
    sd: Option<&mut Storage>,
) -> Option<Calibration> {
    controls::calibration_intro(&mut lcd.lcd, palette).ok()?;
    lcd.set_backlight(true);
    if !wait_release(touch, Duration::from_secs(10)).await {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while touch.sample().is_none() {
        if Instant::now() >= deadline {
            return None;
        }
        Timer::after(Duration::from_millis(25)).await;
    }
    if !wait_release(touch, Duration::from_secs(10)).await {
        return None;
    }
    let targets = [
        Point::new(24, 24),
        Point::new(296, 24),
        Point::new(24, 176),
        Point::new(160, 120),
    ];
    let mut raw = [(0, 0); 4];
    for (i, target) in targets.iter().enumerate() {
        controls::calibration_target(&mut lcd.lcd, palette, *target, i, false).ok()?;
        let deadline = Instant::now() + Duration::from_secs(40);
        let mut stable = 0u8;
        let mut anchor = None;
        loop {
            if Instant::now() >= deadline {
                return None;
            }
            Timer::after(Duration::from_millis(25)).await;
            if let Some(p) = touch.sample() {
                if anchor
                    .is_some_and(|a: (u16, u16)| a.0.abs_diff(p.0) <= 70 && a.1.abs_diff(p.1) <= 70)
                {
                    stable += 1;
                } else {
                    anchor = Some(p);
                    stable = 1;
                }
                if stable >= 5 {
                    raw[i] = p;
                    break;
                }
            } else {
                stable = 0;
                anchor = None;
            }
        }
        controls::calibration_target(&mut lcd.lcd, palette, *target, i, true).ok()?;
        if !wait_release(touch, Duration::from_secs(15)).await {
            return None;
        }
    }
    let Some(calibration) = Calibration::from_points(
        [raw[0], raw[1], raw[2]],
        [targets[0], targets[1], targets[2]],
    ) else {
        let _ = controls::message(
            &mut lcd.lcd,
            palette,
            "Please try again",
            "Touch each cross in its new position",
        );
        Timer::after(Duration::from_secs(2)).await;
        return None;
    };
    let check = calibration.map(raw[3]);
    if !calibration.valid() || (check.x - 160).abs() > 12 || (check.y - 120).abs() > 12 {
        let _ = controls::message(
            &mut lcd.lcd,
            palette,
            "Please try again",
            "Hold each target until it turns green",
        );
        Timer::after(Duration::from_secs(2)).await;
        return None;
    }
    match store.save(calibration, sd) {
        Ok(location) => {
            let _ = controls::message(
                &mut lcd.lcd,
                palette,
                "Touch setup saved",
                match location {
                    SaveLocation::MicroSd | SaveLocation::Both => "Saved to MicroSD",
                    SaveLocation::Flash => "Saved on this device",
                },
            );
        }
        Err(error) => {
            esp_println::println!(
                "Calibration save failed: {:?}; check MicroSD or flash partitions",
                error
            );
            let _ = controls::message(
                &mut lcd.lcd,
                palette,
                "Touch setup complete",
                "Active for this session",
            );
        }
    }
    let c = calibration.0;
    esp_println::println!(
        "[touch]\ncalibration = [{:.8}, {:.8}, {:.8}, {:.8}, {:.8}, {:.8}]",
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5]
    );
    Timer::after(Duration::from_secs(1)).await;
    Some(calibration)
}
