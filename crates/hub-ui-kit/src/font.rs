use iced::Font;
use iced::font::Weight;

pub const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
pub const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");

pub const UI: Font = Font::with_name("Inter");
pub const UI_MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..UI
};

pub const ALL: [&[u8]; 2] = [INTER_REGULAR, INTER_MEDIUM];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fonts_are_truetype() {
        for bytes in ALL {
            assert_eq!(&bytes[..4], &[0, 1, 0, 0]);
            assert!(bytes.len() > 100_000);
        }
    }
}
