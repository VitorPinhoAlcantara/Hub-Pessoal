use std::path::PathBuf;

use rfd::AsyncFileDialog;

pub async fn pick_files() -> Vec<PathBuf> {
    AsyncFileDialog::new()
        .set_title("Escolher arquivos")
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
