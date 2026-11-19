use super::{ConfigError, ErrorKind, bounded::List};
use alloc::{format, string::String};
use core::fmt;
use serde::{
    Deserialize, Deserializer,
    de::{self, Visitor},
};

/// Opaque 24-bit color, independent of display drivers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Color(u32);

impl Color {
    /// Parse exactly `#RRGGBB`; case-insensitive, without whitespace or alpha.
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() != 7
            || !value.starts_with('#')
            || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            return None;
        }
        u32::from_str_radix(&value[1..], 16).ok().map(Self)
    }
    /// Packed `0xRRGGBB` value.
    pub const fn rgb888(self) -> u32 {
        self.0
    }
    /// Truncated RGB565 value for rendering; configuration retains all 24 bits.
    pub const fn rgb565(self) -> u16 {
        (((self.0 >> 19) & 31) << 11 | ((self.0 >> 10) & 63) << 5 | ((self.0 >> 3) & 31)) as u16
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06X}", self.0)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Hex;
        impl Visitor<'_> for Hex {
            type Value = Color;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a #RRGGBB color")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Color, E> {
                Color::parse(value).ok_or_else(|| E::custom("expected #RRGGBB"))
            }
        }
        deserializer.deserialize_str(Hex)
    }
}

/// Theme selection; omitted mode means dark.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// Dark instrument panel.
    #[default]
    Dark,
    /// Light instrument panel.
    Light,
}

/// Resolved UI colors and a fixed-storage chart palette.
#[derive(Clone, Debug)]
pub struct Palette {
    /// Dashboard background.
    pub background: Color,
    /// Widget background.
    pub surface: Color,
    /// Pressed/elevated surface.
    pub raised_surface: Color,
    /// Values and primary text.
    pub text: Color,
    /// Labels and secondary text.
    pub secondary_text: Color,
    /// Borders and grid lines.
    pub border: Color,
    /// Controls and default gauges.
    pub accent: Color,
    /// Success status.
    pub success: Color,
    /// Warning/stale status.
    pub warning: Color,
    /// Error/invalid status.
    pub error: Color,
    chart_colors: [Color; 12],
    chart_len: usize,
}

impl Palette {
    /// Ordered chart colors; one to twelve entries after validation.
    pub fn chart_colors(&self) -> &[Color] {
        &self.chart_colors[..self.chart_len]
    }
    /// Stable slot lookup, cycling through the resolved palette.
    pub fn chart_color(&self, slot: usize) -> Color {
        self.chart_colors[slot % self.chart_len]
    }
    /// Built-in colors from DESIGN.md.
    pub const fn defaults(mode: ThemeMode) -> Self {
        let (ui, chart) = match mode {
            ThemeMode::Dark => (
                [
                    0x111217, 0x181B1F, 0x22252B, 0xCCCCDC, 0x8D8E9A, 0x3C3E45, 0x6E9FFF, 0x6CCF8E,
                    0xFBAD37, 0xFF5286,
                ],
                [0x73BF69, 0xF2CC0C, 0x5794F2, 0xFF9830, 0xF2495C, 0xB877D9],
            ),
            ThemeMode::Light => (
                [
                    0xFBFBFB, 0xFFFFFF, 0xF4F5F5, 0x24292E, 0x5B5F62, 0xBDBFC0, 0x1F62E0, 0x0A764E,
                    0xB04E0C, 0xCF0E5B,
                ],
                [0x37872D, 0x967302, 0x1F60C4, 0xC15C17, 0xC4162A, 0x8F3BB8],
            ),
        };
        Self {
            background: Color(ui[0]),
            surface: Color(ui[1]),
            raised_surface: Color(ui[2]),
            text: Color(ui[3]),
            secondary_text: Color(ui[4]),
            border: Color(ui[5]),
            accent: Color(ui[6]),
            success: Color(ui[7]),
            warning: Color(ui[8]),
            error: Color(ui[9]),
            chart_colors: [
                Color(chart[0]),
                Color(chart[1]),
                Color(chart[2]),
                Color(chart[3]),
                Color(chart[4]),
                Color(chart[5]),
                Color(0),
                Color(0),
                Color(0),
                Color(0),
                Color(0),
                Color(0),
            ],
            chart_len: 6,
        }
    }
}

/// Both palettes are resolved independently, even when one is inactive.
#[derive(Clone, Debug)]
pub struct Theme {
    /// Selected mode.
    pub mode: ThemeMode,
    /// Resolved dark colors.
    pub dark: Palette,
    /// Resolved light colors.
    pub light: Palette,
}

impl Theme {
    /// Active resolved palette.
    pub fn active(&self) -> &Palette {
        match self.mode {
            ThemeMode::Dark => &self.dark,
            ThemeMode::Light => &self.light,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RawTheme {
    mode: ThemeMode,
    dark: RawPalette,
    light: RawPalette,
}

impl RawTheme {
    pub fn resolve(self) -> Result<Theme, ConfigError> {
        Ok(Theme {
            mode: self.mode,
            dark: self.dark.resolve(ThemeMode::Dark)?,
            light: self.light.resolve(ThemeMode::Light)?,
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawPalette {
    background: Option<String>,
    surface: Option<String>,
    raised_surface: Option<String>,
    text: Option<String>,
    secondary_text: Option<String>,
    border: Option<String>,
    accent: Option<String>,
    success: Option<String>,
    warning: Option<String>,
    error: Option<String>,
    chart_colors: Option<List<String, 12>>,
}

impl RawPalette {
    fn resolve(self, mode: ThemeMode) -> Result<Palette, ConfigError> {
        let mut p = Palette::defaults(mode);
        let path = match mode {
            ThemeMode::Dark => "theme.dark",
            ThemeMode::Light => "theme.light",
        };
        macro_rules! overrides { ($($name:ident),*) => { $(if let Some(color) = self.$name { p.$name = required_color(&color, &format!("{path}.{}", stringify!($name)))?; })* }; }
        overrides!(
            background,
            surface,
            raised_surface,
            text,
            secondary_text,
            border,
            accent,
            success,
            warning,
            error
        );
        if let Some(colors) = self.chart_colors {
            if colors.0.is_empty() {
                return Err(ConfigError::new(
                    ErrorKind::Value,
                    &format!(
                        "theme.{}.chart_colors",
                        match mode {
                            ThemeMode::Dark => "dark",
                            ThemeMode::Light => "light",
                        }
                    ),
                    "chart palette must contain 1–12 colors",
                ));
            }
            p.chart_len = colors.0.len();
            for (index, color) in colors.0.iter().enumerate() {
                p.chart_colors[index] =
                    required_color(color, &format!("{path}.chart_colors[{index}]"))?;
            }
        }
        Ok(p)
    }
}

pub(super) fn required_color(value: &str, field: &str) -> Result<Color, ConfigError> {
    Color::parse(value).ok_or_else(|| {
        ConfigError::new(ErrorKind::Value, field, "expected an opaque #RRGGBB color")
    })
}
