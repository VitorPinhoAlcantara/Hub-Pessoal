use std::io::Cursor;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardContent {
    pub files: Vec<PathBuf>,
    pub png: Option<Vec<u8>>,
}

impl ClipboardContent {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.png.is_none()
    }
}

pub fn read() -> ClipboardContent {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return ClipboardContent::default();
    };
    let files = clipboard.get().file_list().unwrap_or_default();
    if !files.is_empty() {
        return ClipboardContent { files, png: None };
    }
    let png = clipboard.get_image().ok().and_then(|image| {
        encode_png(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        )
    });
    ClipboardContent {
        files: Vec::new(),
        png,
    }
}

pub fn encode_png(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let buffer = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut bytes = Cursor::new(Vec::new());
    buffer.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    Some(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_png_produces_png_signature() {
        let png = encode_png(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(encode_png(2, 2, vec![0; 3]).is_none());
    }
}
