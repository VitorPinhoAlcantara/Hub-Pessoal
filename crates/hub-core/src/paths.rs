use std::path::{Path, PathBuf};

use directories::ProjectDirs;

pub const HUB_HOME_ENV: &str = "HUB_HOME";

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("não foi possível determinar a pasta do usuário")]
    NoHomeDir,
    #[error("falha ao criar a pasta {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPaths {
    pub config_dir: PathBuf,
    pub scripts_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl HubPaths {
    pub fn resolve() -> Result<Self, PathsError> {
        let home = std::env::var_os(HUB_HOME_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self::resolve_with(home)
    }

    pub fn resolve_with(hub_home: Option<PathBuf>) -> Result<Self, PathsError> {
        match hub_home {
            Some(home) => Ok(Self::from_home(home)),
            None => Self::from_system(),
        }
    }

    pub fn from_home(home: impl AsRef<Path>) -> Self {
        let home = home.as_ref();
        Self::from_dirs(home.join("config"), home.join("data"))
    }

    fn from_system() -> Result<Self, PathsError> {
        let dirs = ProjectDirs::from("", "", crate::channel::Channel::current().dir_name())
            .ok_or(PathsError::NoHomeDir)?;
        Ok(Self::from_dirs(
            dirs.config_dir().to_path_buf(),
            dirs.data_dir().to_path_buf(),
        ))
    }

    fn from_dirs(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            scripts_dir: config_dir.join("scripts"),
            logs_dir: data_dir.join("logs"),
            config_dir,
            data_dir,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn shortcuts_file(&self) -> PathBuf {
        self.config_dir.join("atalhos.toml")
    }

    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("hub.db")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn ensure_dirs(&self) -> Result<(), PathsError> {
        for dir in [
            &self.config_dir,
            &self.scripts_dir,
            &self.data_dir,
            &self.logs_dir,
        ] {
            std::fs::create_dir_all(dir).map_err(|source| PathsError::CreateDir {
                path: dir.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_home_points_everything_to_one_folder() {
        let home = PathBuf::from("base");
        let paths = HubPaths::resolve_with(Some(home.clone())).unwrap();
        assert_eq!(paths.config_dir, home.join("config"));
        assert_eq!(paths.scripts_dir, home.join("config").join("scripts"));
        assert_eq!(paths.data_dir, home.join("data"));
        assert_eq!(paths.logs_dir, home.join("data").join("logs"));
        assert_eq!(paths.config_file(), home.join("config").join("config.toml"));
        assert_eq!(paths.database_file(), home.join("data").join("hub.db"));
    }

    #[test]
    fn system_paths_use_channel_folder() {
        let name = crate::channel::Channel::current().dir_name();
        let paths = HubPaths::resolve_with(None).unwrap();
        assert!(paths.config_dir.components().any(|c| c.as_os_str() == name));
        assert!(paths.data_dir.components().any(|c| c.as_os_str() == name));
    }

    #[test]
    fn ensure_dirs_creates_all_folders() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path());
        paths.ensure_dirs().unwrap();
        assert!(paths.config_dir.is_dir());
        assert!(paths.scripts_dir.is_dir());
        assert!(paths.data_dir.is_dir());
        assert!(paths.logs_dir.is_dir());
    }
}
