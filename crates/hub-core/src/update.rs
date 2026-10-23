use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backup::{self, BackupError, BackupReason};
use crate::paths::HubPaths;
use crate::platform::{self, PlatformError};

pub const MANIFEST_FILE: &str = "ultima.json";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("erro de entrada e saída: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifesto de atualização inválido: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("versão inválida: {0}")]
    InvalidVersion(String),
    #[error("nome de instalador inválido no manifesto: {0}")]
    InvalidInstaller(String),
    #[error("instalador corrompido: sha256 não confere")]
    ChecksumMismatch,
    #[error("falha ao criar o backup antes de atualizar: {0}")]
    Backup(#[from] BackupError),
    #[error("falha ao iniciar o instalador: {0}")]
    Launch(#[from] PlatformError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    #[serde(rename = "versao")]
    pub version: String,
    #[serde(rename = "instalador")]
    pub installer: String,
    pub sha256: String,
    #[serde(rename = "notas", default)]
    pub notes: String,
    #[serde(rename = "publicado_em", default)]
    pub published_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub installer: PathBuf,
    pub sha256: String,
}

pub fn parse_version(text: &str) -> Result<[u64; 3], UpdateError> {
    let trimmed = text.trim().trim_start_matches(['v', 'V']);
    let mut parts = [0u64; 3];
    for (index, piece) in trimmed.split('.').enumerate() {
        let slot = parts
            .get_mut(index)
            .ok_or_else(|| UpdateError::InvalidVersion(text.to_string()))?;
        *slot = piece
            .parse()
            .map_err(|_| UpdateError::InvalidVersion(text.to_string()))?;
    }
    Ok(parts)
}

pub fn is_newer(candidate: &str, current: &str) -> Result<bool, UpdateError> {
    Ok(parse_version(candidate)? > parse_version(current)?)
}

pub fn check(folder: &Path, current: &str) -> Result<Option<UpdateInfo>, UpdateError> {
    let path = folder.join(MANIFEST_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let manifest: UpdateManifest = serde_json::from_str(&text)?;
    if !is_newer(&manifest.version, current)? {
        return Ok(None);
    }
    let name = Path::new(&manifest.installer);
    if name.file_name().map(Path::new) != Some(name) {
        return Err(UpdateError::InvalidInstaller(manifest.installer));
    }
    Ok(Some(UpdateInfo {
        version: manifest.version.trim().to_string(),
        notes: manifest.notes,
        installer: folder.join(name),
        sha256: manifest.sha256.trim().to_lowercase(),
    }))
}

pub fn file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn apply(paths: &HubPaths, info: &UpdateInfo, now: NaiveDateTime) -> Result<(), UpdateError> {
    if file_sha256(&info.installer)? != info.sha256.trim().to_lowercase() {
        return Err(UpdateError::ChecksumMismatch);
    }
    backup::create(paths, BackupReason::Update, now)?;
    let staging = std::env::temp_dir().join("hub-atualizacao");
    std::fs::create_dir_all(&staging)?;
    let name = info
        .installer
        .file_name()
        .ok_or_else(|| UpdateError::InvalidInstaller(info.installer.display().to_string()))?;
    let copy = staging.join(name);
    std::fs::copy(&info.installer, &copy)?;
    platform::launch_installer(&copy, &std::env::current_exe()?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(folder: &Path, version: &str, installer: &str) {
        let manifest = UpdateManifest {
            version: version.to_string(),
            installer: installer.to_string(),
            sha256: "ab".repeat(32),
            notes: "novidades".to_string(),
            published_at: "2026-10-22T20:00:00".to_string(),
        };
        std::fs::write(
            folder.join(MANIFEST_FILE),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.9").unwrap());
        assert!(is_newer("1.0.0", "0.99.99").unwrap());
        assert!(is_newer("v0.1.1", "0.1.0").unwrap());
        assert!(is_newer("0.2", "0.1.9").unwrap());
        assert!(!is_newer("0.1.0", "0.1.0").unwrap());
        assert!(!is_newer("0.1.0", "0.1.1").unwrap());
        assert!(parse_version("a.b.c").is_err());
        assert!(parse_version("1.2.3.4").is_err());
        assert!(parse_version("").is_err());
    }

    #[test]
    fn check_without_manifest_returns_none() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(check(temp.path(), "0.1.0").unwrap(), None);
        assert_eq!(
            check(&temp.path().join("nao-existe"), "0.1.0").unwrap(),
            None
        );
    }

    #[test]
    fn check_returns_only_newer_versions() {
        let temp = tempfile::tempdir().unwrap();
        write_manifest(temp.path(), "0.1.1", "hub-setup.exe");
        assert_eq!(check(temp.path(), "0.1.2").unwrap(), None);
        assert_eq!(check(temp.path(), "0.1.1").unwrap(), None);
        let info = check(temp.path(), "0.1.0").unwrap().unwrap();
        assert_eq!(info.version, "0.1.1");
        assert_eq!(info.notes, "novidades");
        assert_eq!(info.installer, temp.path().join("hub-setup.exe"));
        assert_eq!(info.sha256, "ab".repeat(32));
    }

    #[test]
    fn check_rejects_installer_with_path() {
        let temp = tempfile::tempdir().unwrap();
        write_manifest(temp.path(), "0.1.1", "sub/setup.exe");
        assert!(matches!(
            check(temp.path(), "0.1.0"),
            Err(UpdateError::InvalidInstaller(_))
        ));
    }

    #[test]
    fn sha256_matches_known_digest() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.bin");
        std::fs::write(&file, "abc").unwrap();
        assert_eq!(
            file_sha256(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn apply_with_wrong_sha256_fails_before_anything_else() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path().join("home"));
        let installer = temp.path().join("setup.exe");
        std::fs::write(&installer, "conteudo").unwrap();
        let info = UpdateInfo {
            version: "0.1.1".to_string(),
            notes: String::new(),
            installer,
            sha256: "00".repeat(32),
        };
        let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 22)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        assert!(matches!(
            apply(&paths, &info, now),
            Err(UpdateError::ChecksumMismatch)
        ));
        assert!(!paths.backups_dir().exists());
    }
}
