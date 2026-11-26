use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backup::{self, BackupError, BackupReason};
use crate::paths::HubPaths;
use crate::platform::{self, PlatformError};

pub const MANIFEST_FILE: &str = "ultima.json";
const RELEASE_URL: &str =
    "https://github.com/VitorPinhoAlcantara/Hub-Pessoal/releases/latest/download";
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

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
    #[error("falha ao acessar o GitHub: {0}")]
    Http(#[from] Box<ureq::Error>),
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
pub enum UpdateSource {
    Folder(PathBuf),
    GitHub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installer {
    Local(PathBuf),
    Remote { url: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub installer: Installer,
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

pub fn check(source: &UpdateSource, current: &str) -> Result<Option<UpdateInfo>, UpdateError> {
    let text = match source {
        UpdateSource::Folder(folder) => match std::fs::read_to_string(folder.join(MANIFEST_FILE)) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        },
        UpdateSource::GitHub => download_manifest()?,
    };
    match text {
        Some(text) => evaluate(&text, source, current),
        None => Ok(None),
    }
}

fn download_manifest() -> Result<Option<String>, UpdateError> {
    let response = ureq::get(format!("{RELEASE_URL}/{MANIFEST_FILE}"))
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .call();
    match response {
        Ok(mut response) => Ok(Some(
            response.body_mut().read_to_string().map_err(Box::new)?,
        )),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(error) => Err(Box::new(error).into()),
    }
}

fn evaluate(
    text: &str,
    source: &UpdateSource,
    current: &str,
) -> Result<Option<UpdateInfo>, UpdateError> {
    let manifest: UpdateManifest = serde_json::from_str(text)?;
    if !is_newer(&manifest.version, current)? {
        return Ok(None);
    }
    let name = Path::new(&manifest.installer);
    let plain = manifest
        .installer
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !plain || name.file_name().map(Path::new) != Some(name) {
        return Err(UpdateError::InvalidInstaller(manifest.installer));
    }
    let installer = match source {
        UpdateSource::Folder(folder) => Installer::Local(folder.join(name)),
        UpdateSource::GitHub => Installer::Remote {
            url: format!("{RELEASE_URL}/{}", manifest.installer),
            name: manifest.installer.clone(),
        },
    };
    Ok(Some(UpdateInfo {
        version: manifest.version.trim().to_string(),
        notes: manifest.notes,
        installer,
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
    let staging = std::env::temp_dir().join("hub-atualizacao");
    std::fs::create_dir_all(&staging)?;
    let copy = match &info.installer {
        Installer::Local(path) => {
            let name = path
                .file_name()
                .ok_or_else(|| UpdateError::InvalidInstaller(path.display().to_string()))?;
            let copy = staging.join(name);
            std::fs::copy(path, &copy)?;
            copy
        }
        Installer::Remote { url, name } => {
            let copy = staging.join(name);
            download_installer(url, &copy)?;
            copy
        }
    };
    if file_sha256(&copy)? != info.sha256.trim().to_lowercase() {
        let _ = std::fs::remove_file(&copy);
        return Err(UpdateError::ChecksumMismatch);
    }
    backup::create(paths, BackupReason::Update, now)?;
    platform::launch_installer(&copy, &std::env::current_exe()?)?;
    Ok(())
}

fn download_installer(url: &str, target: &Path) -> Result<(), UpdateError> {
    let mut response = ureq::get(url)
        .config()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(30 * 60)))
        .build()
        .call()
        .map_err(Box::new)?;
    let mut body = response.body_mut().as_reader().take(MAX_INSTALLER_BYTES);
    let mut file = std::fs::File::create(target)?;
    std::io::copy(&mut body, &mut file)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &Path) -> UpdateSource {
        UpdateSource::Folder(path.to_path_buf())
    }

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
        assert_eq!(check(&folder(temp.path()), "0.1.0").unwrap(), None);
        assert_eq!(
            check(&folder(&temp.path().join("nao-existe")), "0.1.0").unwrap(),
            None
        );
    }

    #[test]
    fn check_returns_only_newer_versions() {
        let temp = tempfile::tempdir().unwrap();
        write_manifest(temp.path(), "0.1.1", "hub-setup.exe");
        assert_eq!(check(&folder(temp.path()), "0.1.2").unwrap(), None);
        assert_eq!(check(&folder(temp.path()), "0.1.1").unwrap(), None);
        let info = check(&folder(temp.path()), "0.1.0").unwrap().unwrap();
        assert_eq!(info.version, "0.1.1");
        assert_eq!(info.notes, "novidades");
        assert_eq!(
            info.installer,
            Installer::Local(temp.path().join("hub-setup.exe"))
        );
        assert_eq!(info.sha256, "ab".repeat(32));
    }

    #[test]
    fn check_rejects_installer_with_path() {
        let temp = tempfile::tempdir().unwrap();
        write_manifest(temp.path(), "0.1.1", "sub/setup.exe");
        assert!(matches!(
            check(&folder(temp.path()), "0.1.0"),
            Err(UpdateError::InvalidInstaller(_))
        ));
    }

    #[test]
    fn github_manifest_points_to_the_release_asset() {
        let text = r#"{"versao":"0.2.0","instalador":"Hub-0.2.0-setup.exe","sha256":"AB"}"#;
        let info = evaluate(text, &UpdateSource::GitHub, "0.1.3")
            .unwrap()
            .unwrap();
        assert_eq!(
            info.installer,
            Installer::Remote {
                url: format!("{RELEASE_URL}/Hub-0.2.0-setup.exe"),
                name: "Hub-0.2.0-setup.exe".to_string(),
            }
        );
        assert_eq!(info.sha256, "ab");
        let unsafe_name = text.replace("Hub-0.2.0-setup.exe", "../a?b.exe");
        assert!(matches!(
            evaluate(&unsafe_name, &UpdateSource::GitHub, "0.1.3"),
            Err(UpdateError::InvalidInstaller(_))
        ));
    }

    #[test]
    #[ignore]
    fn github_release_is_reachable() {
        let info = check(&UpdateSource::GitHub, "0.0.1").unwrap().unwrap();
        let Installer::Remote { url, name } = &info.installer else {
            panic!("a origem deveria ser remota");
        };
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join(name);
        download_installer(url, &target).unwrap();
        assert_eq!(file_sha256(&target).unwrap(), info.sha256);
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
            installer: Installer::Local(installer),
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
