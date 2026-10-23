use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub use image::RgbaImage;
use image::imageops::FilterType;

use crate::paths::HubPaths;

const CACHE_DIR: &str = "papel-de-parede";
const FILE_PREFIX: &str = "base-";
const FILE_EXTENSION: &str = ".png";
const PARTIAL_EXTENSION: &str = "parcial";
const MAX_SIDE: u32 = 960;

#[derive(Debug, thiserror::Error)]
pub enum WallpaperError {
    #[error("erro de entrada e saída: {0}")]
    Io(#[from] std::io::Error),
    #[error("falha ao processar a imagem: {0}")]
    Image(#[from] image::ImageError),
}

pub fn cache_dir(paths: &HubPaths) -> PathBuf {
    paths.data_dir.join(CACHE_DIR)
}

fn cache_key(source: &Path) -> Result<String, WallpaperError> {
    let metadata = fs::metadata(source)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

pub fn source_key(source: &Path) -> Result<String, WallpaperError> {
    cache_key(source)
}

pub fn load_base(paths: &HubPaths, source: &Path) -> Result<RgbaImage, WallpaperError> {
    let dir = cache_dir(paths);
    fs::create_dir_all(&dir)?;
    let name = format!("{FILE_PREFIX}{}{FILE_EXTENSION}", cache_key(source)?);
    let target = dir.join(&name);
    if target.exists()
        && let Ok(cached) = image::open(&target)
    {
        return Ok(cached.to_rgba8());
    }
    let decoded = image::ImageReader::open(source)?
        .with_guessed_format()?
        .decode()?;
    let reduced = decoded
        .resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle)
        .to_rgba8();
    let partial = dir.join(format!(
        "{name}.{:?}.{PARTIAL_EXTENSION}",
        std::thread::current().id()
    ));
    reduced.save_with_format(&partial, image::ImageFormat::Png)?;
    if fs::rename(&partial, &target).is_err() {
        let _ = fs::remove_file(&partial);
    }
    remove_others(&dir, &name);
    Ok(reduced)
}

pub fn blurred(base: &RgbaImage, blur: u32) -> RgbaImage {
    if blur == 0 {
        base.clone()
    } else {
        image::imageops::fast_blur(base, blur as f32 / 2.0)
    }
}

fn remove_others(dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name != keep
            && file_name.starts_with(FILE_PREFIX)
            && file_name.ends_with(FILE_EXTENSION)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path, width: u32, height: u32) -> PathBuf {
        let path = dir.join("origem.png");
        let buffer = image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        buffer.save(&path).unwrap();
        path
    }

    #[test]
    fn base_is_reduced_cached_and_reused() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path().join("home"));
        let source = sample(temp.path(), 1600, 800);
        let first = load_base(&paths, &source).unwrap();
        assert_eq!(first.width(), MAX_SIDE);
        assert!(first.height() <= MAX_SIDE);
        let cached: Vec<_> = fs::read_dir(cache_dir(&paths)).unwrap().flatten().collect();
        assert_eq!(cached.len(), 1);
        let stamp = cached[0].metadata().unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let second = load_base(&paths, &source).unwrap();
        assert_eq!(first.dimensions(), second.dimensions());
        assert_eq!(cached[0].metadata().unwrap().modified().unwrap(), stamp);
    }

    #[test]
    fn blur_keeps_size_and_changes_pixels() {
        let base = RgbaImage::from_fn(64, 32, |x, _| {
            image::Rgba([if x < 32 { 0 } else { 255 }, 0, 0, 255])
        });
        assert_eq!(blurred(&base, 0), base);
        let soft = blurred(&base, 20);
        assert_eq!(soft.dimensions(), base.dimensions());
        assert_ne!(soft, base);
    }

    #[test]
    fn missing_source_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path().join("home"));
        let result = load_base(&paths, &temp.path().join("nada.png"));
        assert!(matches!(result, Err(WallpaperError::Io(_))));
    }
}
