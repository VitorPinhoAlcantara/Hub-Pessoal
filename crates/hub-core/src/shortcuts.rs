use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::per_os::PerOs;
use crate::platform::{Os, Platform, PlatformError};

pub const DEFAULT_GROUP: &str = "Geral";

#[derive(Debug, thiserror::Error)]
pub enum ShortcutError {
    #[error("erro de arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("falha ao gravar os atalhos: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("atalhos.toml inválido: {0}")]
    Invalid(String),
    #[error("o nome do atalho não pode ficar vazio")]
    EmptyName,
    #[error("informe o alvo do atalho")]
    EmptyTarget,
    #[error("atalho {0} não encontrado")]
    NotFound(usize),
    #[error("este atalho não tem alvo para este sistema")]
    NoTarget,
    #[error("{0}")]
    Platform(#[from] PlatformError),
}

pub type Result<T> = std::result::Result<T, ShortcutError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShortcutKind {
    #[serde(rename = "programa")]
    Program,
    #[serde(rename = "pasta")]
    Folder,
    #[serde(rename = "url")]
    Url,
    #[serde(rename = "comando")]
    Command,
}

impl ShortcutKind {
    pub const ALL: [ShortcutKind; 4] = [
        ShortcutKind::Program,
        ShortcutKind::Folder,
        ShortcutKind::Url,
        ShortcutKind::Command,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ShortcutKind::Program => "Programa",
            ShortcutKind::Folder => "Pasta",
            ShortcutKind::Url => "Link",
            ShortcutKind::Command => "Comando",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "programa" => Some(ShortcutKind::Program),
            "pasta" => Some(ShortcutKind::Folder),
            "url" => Some(ShortcutKind::Url),
            "comando" => Some(ShortcutKind::Command),
            _ => None,
        }
    }
}

fn default_group() -> String {
    DEFAULT_GROUP.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    #[serde(rename = "nome")]
    pub name: String,
    #[serde(rename = "tipo")]
    pub kind: ShortcutKind,
    #[serde(rename = "alvo")]
    pub target: PerOs,
    #[serde(rename = "grupo", default = "default_group")]
    pub group: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ShortcutsFile {
    #[serde(rename = "atalho", default)]
    items: Vec<Shortcut>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub shortcuts: Vec<Shortcut>,
    pub problem: Option<String>,
}

#[derive(Default)]
pub struct ShortcutStore {
    last_valid: Option<Vec<Shortcut>>,
}

impl ShortcutStore {
    pub fn load(&mut self, path: &Path) -> Loaded {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded::default();
            }
            Err(error) => {
                return Loaded {
                    shortcuts: self.last_valid.clone().unwrap_or_default(),
                    problem: Some(error.to_string()),
                };
            }
        };
        match toml::from_str::<ShortcutsFile>(&text) {
            Ok(file) => {
                self.last_valid = Some(file.items.clone());
                Loaded {
                    shortcuts: file.items,
                    problem: None,
                }
            }
            Err(error) => {
                tracing::warn!(%error, "atalhos.toml inválido");
                Loaded {
                    shortcuts: self.last_valid.clone().unwrap_or_default(),
                    problem: Some(format!(
                        "atalhos.toml inválido, mostrando a última versão válida: {error}"
                    )),
                }
            }
        }
    }

    fn editable(&mut self, path: &Path) -> Result<Vec<Shortcut>> {
        let loaded = self.load(path);
        match loaded.problem {
            Some(problem) => Err(ShortcutError::Invalid(problem)),
            None => Ok(loaded.shortcuts),
        }
    }

    pub fn add(&mut self, path: &Path, shortcut: Shortcut) -> Result<usize> {
        let shortcut = validate(shortcut)?;
        let mut items = self.editable(path)?;
        items.push(shortcut);
        save(path, &items)?;
        self.last_valid = Some(items.clone());
        Ok(items.len() - 1)
    }

    pub fn update(&mut self, path: &Path, index: usize, shortcut: Shortcut) -> Result<()> {
        let shortcut = validate(shortcut)?;
        let mut items = self.editable(path)?;
        let slot = items.get_mut(index).ok_or(ShortcutError::NotFound(index))?;
        *slot = shortcut;
        save(path, &items)?;
        self.last_valid = Some(items);
        Ok(())
    }

    pub fn remove(&mut self, path: &Path, index: usize) -> Result<Shortcut> {
        let mut items = self.editable(path)?;
        if index >= items.len() {
            return Err(ShortcutError::NotFound(index));
        }
        let removed = items.remove(index);
        save(path, &items)?;
        self.last_valid = Some(items);
        Ok(removed)
    }
}

fn validate(mut shortcut: Shortcut) -> Result<Shortcut> {
    shortcut.name = shortcut.name.trim().to_string();
    if shortcut.name.is_empty() {
        return Err(ShortcutError::EmptyName);
    }
    if shortcut.target.windows().is_none() && shortcut.target.linux().is_none() {
        return Err(ShortcutError::EmptyTarget);
    }
    shortcut.group = shortcut.group.trim().to_string();
    if shortcut.group.is_empty() {
        shortcut.group = default_group();
    }
    Ok(shortcut)
}

fn save(path: &Path, items: &[Shortcut]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = ShortcutsFile {
        items: items.to_vec(),
    };
    std::fs::write(path, toml::to_string_pretty(&file)?)?;
    Ok(())
}

pub fn groups(shortcuts: &[Shortcut]) -> Vec<String> {
    let mut groups: Vec<String> = Vec::new();
    for shortcut in shortcuts {
        if !groups.contains(&shortcut.group) {
            groups.push(shortcut.group.clone());
        }
    }
    groups
}

pub fn matches(shortcut: &Shortcut, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || shortcut.name.to_lowercase().contains(&query)
        || shortcut.group.to_lowercase().contains(&query)
}

pub fn expand_home(target: &str) -> PathBuf {
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    match (target.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => PathBuf::from(target),
    }
}

pub fn normalize_url(target: &str) -> String {
    let target = target.trim();
    if target.contains("://") || target.starts_with("mailto:") {
        target.to_string()
    } else {
        format!("https://{target}")
    }
}

pub fn open(platform: &dyn Platform, shortcut: &Shortcut) -> Result<()> {
    let os: Os = platform.os();
    let target = shortcut.target.for_os(os).ok_or(ShortcutError::NoTarget)?;
    match shortcut.kind {
        ShortcutKind::Program => platform.open_program(target)?,
        ShortcutKind::Folder => platform.open_path(&expand_home(target))?,
        ShortcutKind::Url => platform.open_url(&normalize_url(target))?,
        ShortcutKind::Command => platform.run_command(target)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{FakePlatform, PlatformCall};

    fn shortcut(name: &str, kind: ShortcutKind, target: &str, group: &str) -> Shortcut {
        Shortcut {
            name: name.into(),
            kind,
            target: PerOs::Same(target.into()),
            group: group.into(),
        }
    }

    #[test]
    fn reads_spec_example() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("atalhos.toml");
        std::fs::write(
            &path,
            "[[atalho]]\nnome = \"VS Code\"\ntipo = \"programa\"\nalvo = { windows = \"code\", linux = \"code\" }\ngrupo = \"Dev\"\n\n[[atalho]]\nnome = \"Projetos\"\ntipo = \"pasta\"\nalvo = { windows = \"C:\\\\Projetos\", linux = \"~/projetos\" }\ngrupo = \"Pastas\"\n",
        )
        .unwrap();
        let loaded = ShortcutStore::default().load(&path);
        assert!(loaded.problem.is_none());
        assert_eq!(loaded.shortcuts.len(), 2);
        assert_eq!(loaded.shortcuts[1].target.windows(), Some("C:\\Projetos"));
        assert_eq!(groups(&loaded.shortcuts), ["Dev", "Pastas"]);
    }

    #[test]
    fn missing_file_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let loaded = ShortcutStore::default().load(&temp.path().join("atalhos.toml"));
        assert_eq!(loaded, Loaded::default());
    }

    #[test]
    fn add_update_remove_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("atalhos.toml");
        let mut store = ShortcutStore::default();
        store
            .add(
                &path,
                shortcut(" Site ", ShortcutKind::Url, "exemplo.com", " "),
            )
            .unwrap();
        store
            .add(
                &path,
                shortcut("Docs", ShortcutKind::Folder, "~/docs", "Pastas"),
            )
            .unwrap();
        let loaded = store.load(&path);
        assert_eq!(loaded.shortcuts[0].name, "Site");
        assert_eq!(loaded.shortcuts[0].group, DEFAULT_GROUP);
        store
            .update(
                &path,
                0,
                shortcut("Site 2", ShortcutKind::Url, "x.com", "Web"),
            )
            .unwrap();
        assert_eq!(store.load(&path).shortcuts[0].group, "Web");
        let removed = store.remove(&path, 1).unwrap();
        assert_eq!(removed.name, "Docs");
        assert_eq!(store.load(&path).shortcuts.len(), 1);
        assert!(matches!(
            store.remove(&path, 5),
            Err(ShortcutError::NotFound(5))
        ));
    }

    #[test]
    fn invalid_file_keeps_last_valid_and_blocks_edits() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("atalhos.toml");
        let mut store = ShortcutStore::default();
        store
            .add(&path, shortcut("A", ShortcutKind::Url, "a.com", "G"))
            .unwrap();
        std::fs::write(&path, "[[atalho]\nnome =").unwrap();
        let loaded = store.load(&path);
        assert_eq!(loaded.shortcuts.len(), 1);
        assert!(loaded.problem.is_some());
        assert!(matches!(
            store.add(&path, shortcut("B", ShortcutKind::Url, "b.com", "G")),
            Err(ShortcutError::Invalid(_))
        ));
    }

    #[test]
    fn validation_rejects_empty_values() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("atalhos.toml");
        let mut store = ShortcutStore::default();
        assert!(matches!(
            store.add(&path, shortcut(" ", ShortcutKind::Url, "a", "")),
            Err(ShortcutError::EmptyName)
        ));
        assert!(matches!(
            store.add(&path, shortcut("x", ShortcutKind::Url, "", "")),
            Err(ShortcutError::EmptyTarget)
        ));
    }

    #[test]
    fn open_dispatches_by_kind() {
        let platform = FakePlatform::new(Os::Windows);
        open(&platform, &shortcut("a", ShortcutKind::Program, "code", "")).unwrap();
        open(
            &platform,
            &shortcut("b", ShortcutKind::Url, "exemplo.com", ""),
        )
        .unwrap();
        open(
            &platform,
            &shortcut("c", ShortcutKind::Command, "echo oi", ""),
        )
        .unwrap();
        open(&platform, &shortcut("d", ShortcutKind::Folder, "C:\\x", "")).unwrap();
        assert_eq!(
            platform.calls(),
            vec![
                PlatformCall::OpenProgram("code".into()),
                PlatformCall::OpenUrl("https://exemplo.com".into()),
                PlatformCall::RunCommand("echo oi".into()),
                PlatformCall::OpenPath(PathBuf::from("C:\\x")),
            ]
        );
        let linux_only = Shortcut {
            target: PerOs::split(None, Some("x".into())),
            ..shortcut("e", ShortcutKind::Program, "", "")
        };
        assert!(matches!(
            open(&platform, &linux_only),
            Err(ShortcutError::NoTarget)
        ));
    }

    #[test]
    fn helpers_expand_home_and_match() {
        assert!(!expand_home("~/projetos").starts_with("~"));
        assert_eq!(expand_home("/tmp/x"), PathBuf::from("/tmp/x"));
        assert_eq!(normalize_url("http://a"), "http://a");
        let s = shortcut("VS Code", ShortcutKind::Program, "code", "Dev");
        assert!(matches(&s, "vs"));
        assert!(matches(&s, "dev"));
        assert!(!matches(&s, "zzz"));
    }
}
