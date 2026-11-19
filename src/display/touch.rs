use embedded_graphics::prelude::Point;
/// Affine transform from raw XPT2046 coordinates to landscape pixels.
#[derive(Clone, Copy, Debug)]
pub struct Calibration(pub [f64; 6]);
impl Default for Calibration {
    fn default() -> Self {
        // Board starting estimate. Use three-point calibration on the panel for
        // exact bounds and axis direction; resistive panels vary between units.
        Self([
            320.0 / 3700.0,
            0.0,
            -200.0 * 320.0 / 3700.0,
            0.0,
            240.0 / 3700.0,
            -200.0 * 240.0 / 3700.0,
        ])
    }
}
impl Calibration {
    pub fn map(&self, (x, y): (u16, u16)) -> Point {
        let c = self.0;
        Point::new(
            (c[0] * f64::from(x) + c[1] * f64::from(y) + c[2]) as i32,
            (c[3] * f64::from(x) + c[4] * f64::from(y) + c[5]) as i32,
        )
    }
    pub fn from_points(raw: [(u16, u16); 3], screen: [Point; 3]) -> Option<Self> {
        let [(x1, y1), (x2, y2), (x3, y3)] = raw.map(|(x, y)| (f64::from(x), f64::from(y)));
        let det = (x1 - x3) * (y2 - y3) - (x2 - x3) * (y1 - y3);
        if det.abs() < 100.0 {
            return None;
        }
        let solve = |v: [f64; 3]| {
            let a = ((v[0] - v[2]) * (y2 - y3) - (v[1] - v[2]) * (y1 - y3)) / det;
            let b = ((x1 - x3) * (v[1] - v[2]) - (x2 - x3) * (v[0] - v[2])) / det;
            [a, b, v[2] - a * x3 - b * y3]
        };
        let a = solve(screen.map(|p| f64::from(p.x)));
        let b = solve(screen.map(|p| f64::from(p.y)));
        Some(Self([a[0], a[1], a[2], b[0], b[1], b[2]]))
    }
}

impl Calibration {
    pub fn valid(&self) -> bool {
        let c = self.0;
        c.iter().all(|v| v.is_finite() && v.abs() <= 32768.0)
            && (c[0] * c[4] - c[1] * c[3]).abs() >= 1e-9
    }
    /// Versioned record with checksum. Two independent erase sectors preserve
    /// the prior valid calibration if power fails while saving the replacement.
    pub fn encode(&self, generation: u32) -> [u8; 64] {
        let mut record = [0; 64];
        record[..8].copy_from_slice(b"GRAPHT01");
        record[8..12].copy_from_slice(&generation.to_le_bytes());
        for (i, v) in self.0.iter().enumerate() {
            record[12 + i * 8..20 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        let checksum = checksum(&record[..60]);
        record[60..].copy_from_slice(&checksum.to_le_bytes());
        record
    }
    pub fn decode(record: &[u8; 64]) -> Option<(u32, Self)> {
        if &record[..8] != b"GRAPHT01"
            || checksum(&record[..60]) != u32::from_le_bytes(record[60..64].try_into().ok()?)
        {
            return None;
        }
        let mut c = [0.0; 6];
        for (i, v) in c.iter_mut().enumerate() {
            *v = f64::from_le_bytes(record[12 + i * 8..20 + i * 8].try_into().ok()?);
        }
        let calibration = Self(c);
        if !calibration.valid() {
            return None;
        }
        Some((
            u32::from_le_bytes(record[8..12].try_into().ok()?),
            calibration,
        ))
    }
}
fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x01000193)
    })
}

pub(crate) fn newest(r: [Option<(u32, Calibration)>; 2]) -> Option<usize> {
    match (r[0], r[1]) {
        (Some((a, _)), Some((b, _))) => Some(if b.wrapping_sub(a) < (1 << 31) { 1 } else { 0 }),
        (Some(_), None) => Some(0),
        (None, Some(_)) => Some(1),
        _ => None,
    }
}
