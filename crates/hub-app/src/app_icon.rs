use hub_ui_kit::tokens::{Palette, hex};

const ACCENT: hub_ui_kit::iced::Color = hex(0x7C9CF5);
const BG_BASE: hub_ui_kit::iced::Color = Palette::DARK.background;

pub fn rgba(size: u32) -> Vec<u8> {
    let to_rgba = |c: hub_ui_kit::iced::Color| {
        [
            (c.r * 255.0) as u8,
            (c.g * 255.0) as u8,
            (c.b * 255.0) as u8,
            255,
        ]
    };
    let accent = to_rgba(ACCENT);
    let dark = to_rgba(BG_BASE);
    let s = size as f32;
    let radius = s * 0.24;
    let inner = s * 0.18;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let inside = rounded_rect(fx, fy, 0.0, 0.0, s, s, radius);
            let pixel = if !inside {
                [0, 0, 0, 0]
            } else if is_glyph(fx / s, fy / s, inner / s) {
                dark
            } else {
                accent
            };
            pixels.extend_from_slice(&pixel);
        }
    }
    pixels
}

fn rounded_rect(x: f32, y: f32, left: f32, top: f32, width: f32, height: f32, r: f32) -> bool {
    let cx = x.clamp(left + r, left + width - r);
    let cy = y.clamp(top + r, top + height - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

fn is_glyph(u: f32, v: f32, stroke: f32) -> bool {
    let left_bar = (0.28..0.28 + stroke).contains(&u) && (0.22..0.78).contains(&v);
    let right_bar = (0.72 - stroke..0.72).contains(&u) && (0.42..0.78).contains(&v);
    let arch = (0.28..0.72).contains(&u) && (0.42..0.42 + stroke).contains(&v);
    left_bar || right_bar || arch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_size_and_transparent_corners() {
        let pixels = rgba(32);
        assert_eq!(pixels.len(), 32 * 32 * 4);
        assert_eq!(pixels[3], 0);
        let center = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(pixels[center + 3], 255);
    }
}
