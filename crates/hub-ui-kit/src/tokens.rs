use iced::Color;

const fn hex(value: u32) -> Color {
    Color::from_rgb8(
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    )
}

pub const BG_BASE: Color = hex(0x111214);
pub const BG_PANEL: Color = hex(0x1A1B1E);
pub const BG_ELEVATED: Color = hex(0x232428);
pub const BG_HOVER: Color = hex(0x2B2D31);
pub const BORDER_SUBTLE: Color = hex(0x2E3035);
pub const TEXT_PRIMARY: Color = hex(0xE6E6E7);
pub const TEXT_SECONDARY: Color = hex(0x9A9BA1);
pub const ACCENT: Color = hex(0x7C9CF5);
pub const DANGER: Color = hex(0xE5676B);
pub const SUCCESS: Color = hex(0x6FCF97);
pub const WARNING: Color = hex(0xE5B567);

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

pub fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}
