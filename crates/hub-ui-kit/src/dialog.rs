use std::path::PathBuf;

use rfd::AsyncFileDialog;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

pub async fn pick_images() -> Vec<PathBuf> {
    AsyncFileDialog::new()
        .set_title("Escolher imagens")
        .add_filter("Imagens", IMAGE_EXTENSIONS)
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect()
}

pub async fn pick_file(title: &str) -> Option<PathBuf> {
    AsyncFileDialog::new()
        .set_title(title)
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

pub async fn pick_folder(title: &str) -> Option<PathBuf> {
    AsyncFileDialog::new()
        .set_title(title)
        .pick_folder()
        .await
        .map(|file| file.path().to_path_buf())
}
