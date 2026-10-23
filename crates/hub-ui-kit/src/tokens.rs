use std::sync::RwLock;

use iced::Color;

pub const fn hex(value: u32) -> Color {
    Color::from_rgb8(
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub surface: Color,
    pub text: Color,
    pub accent: Color,
    pub secondary: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
}

impl Palette {
    pub const DARK: Palette = Palette {
        background: hex(0x111214),
        surface: hex(0x1A1B1E),
        text: hex(0xE6E6E7),
        accent: hex(0x7C9CF5),
        secondary: hex(0xB48CF2),
        danger: hex(0xE5676B),
        success: hex(0x6FCF97),
        warning: hex(0xE5B567),
    };

    pub const LIGHT: Palette = Palette {
        background: hex(0xE9EAEE),
        surface: hex(0xFFFFFF),
        text: hex(0x1C1D21),
        accent: hex(0x4C6FE0),
        secondary: hex(0x8B5CD6),
        danger: hex(0xD0454A),
        success: hex(0x2E9E5E),
        warning: hex(0xB8862B),
    };

    pub fn is_light(&self) -> bool {
        luminance(self.background) > 0.5
    }
}

static PALETTE: RwLock<Palette> = RwLock::new(Palette::DARK);
static TRANSLUCENCY: RwLock<Option<f32>> = RwLock::new(None);

pub fn set_translucency(dim: Option<f32>) {
    if let Ok(mut current) = TRANSLUCENCY.write() {
        *current = dim.map(|value| value.clamp(0.0, 0.95));
    }
}

pub fn translucency() -> Option<f32> {
    TRANSLUCENCY.read().ok().and_then(|value| *value)
}

fn surface() -> Color {
    let p = palette();
    match translucency() {
        Some(dim) => alpha(p.surface, (dim + 0.3).min(0.94)),
        None => p.surface,
    }
}

pub fn set_palette(palette: Palette) {
    if let Ok(mut current) = PALETTE.write() {
        *current = palette;
    }
}

pub fn palette() -> Palette {
    PALETTE.read().map(|p| *p).unwrap_or(Palette::DARK)
}

pub fn mix(a: Color, b: Color, amount: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * amount,
        g: a.g + (b.g - a.g) * amount,
        b: a.b + (b.b - a.b) * amount,
        a: a.a + (b.a - a.a) * amount,
    }
}

pub fn luminance(color: Color) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}

pub fn contrast_text(background: Color) -> Color {
    if luminance(background) > 0.55 {
        hex(0x111214)
    } else {
        hex(0xFFFFFF)
    }
}

pub fn bg_base() -> Color {
    let p = palette();
    match translucency() {
        Some(dim) => alpha(p.background, dim),
        None => p.background,
    }
}

pub fn bg_panel() -> Color {
    surface()
}

pub fn bg_elevated() -> Color {
    mix(surface(), palette().text, 0.05)
}

pub fn bg_hover() -> Color {
    mix(surface(), palette().text, 0.09)
}

pub fn border_subtle() -> Color {
    mix(surface(), palette().text, 0.11)
}

pub fn text_primary() -> Color {
    palette().text
}

pub fn text_secondary() -> Color {
    let p = palette();
    mix(p.text, p.surface, 0.38)
}

pub fn accent() -> Color {
    palette().accent
}

pub fn on_accent() -> Color {
    contrast_text(palette().accent)
}

pub fn danger() -> Color {
    palette().danger
}

pub fn success() -> Color {
    palette().success
}

pub fn warning() -> Color {
    palette().warning
}

pub fn console_bg() -> Color {
    let p = palette();
    let color = mix(p.background, hex(0x000000), 0.25);
    match translucency() {
        Some(dim) => alpha(color, (dim + 0.35).min(0.96)),
        None => color,
    }
}

pub fn kind_appointment() -> Color {
    palette().accent
}

pub fn kind_routine() -> Color {
    palette().secondary
}

pub fn kind_reminder() -> Color {
    palette().warning
}

pub const RADIUS_PANEL: f32 = 10.0;
pub const RADIUS_CONTROL: f32 = 6.0;
pub const GAP_PANEL: f32 = 8.0;
pub const SPACE_UNIT: f32 = 4.0;

pub const FONT_SIZE: f32 = 13.0;
pub const FONT_SIZE_SMALL: f32 = 11.0;
pub const FONT_SIZE_TITLE: f32 = 18.0;
pub const FONT_SIZE_HEADING: f32 = 14.0;

pub const TOOLBAR_HEIGHT: f32 = 40.0;
pub const SIDEBAR_WIDTH: f32 = 56.0;
pub const ICON_SIZE: f32 = 18.0;

pub const fn units(count: u16) -> f32 {
    SPACE_UNIT * count as f32
}

pub fn solid(color: Color) -> Color {
    Color { a: 1.0, ..color }
}

pub fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}
