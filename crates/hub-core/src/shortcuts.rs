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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ShortcutsFile {
    #[serde(rename = "recolhidos", default, skip_serializing_if = "Vec::is_empty")]
    collapsed: Vec<String>,
    #[serde(rename = "atalho", default)]
    items: Vec<Shortcut>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub shortcuts: Vec<Shortcut>,
    pub collapsed: Vec<String>,
    pub problem: Option<String>,
}

impl Loaded {
    fn valid(file: ShortcutsFile) -> Self {
        Self {
            shortcuts: file.items,
            collapsed: file.collapsed,
            problem: None,
        }
    }

    fn stale(file: Option<ShortcutsFile>, problem: String) -> Self {
        let file = file.unwrap_or_default();
        Self {
            shortcuts: file.items,
            collapsed: file.collapsed,
            problem: Some(problem),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveTarget {
    Shortcut(usize),
    GroupEnd(String),
}

#[derive(Default)]
pub struct ShortcutStore {
    last_valid: Option<ShortcutsFile>,
}

impl ShortcutStore {
    pub fn load(&mut self, path: &Path) -> Loaded {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Loaded::default();
            }
            Err(error) => {
                return Loaded::stale(self.last_valid.clone(), error.to_string());
            }
        };
        match toml::from_str::<ShortcutsFile>(&text) {
            Ok(file) => {
                self.last_valid = Some(file.clone());
                Loaded::valid(file)
            }
            Err(error) => {
                tracing::warn!(%error, "atalhos.toml inválido");
                Loaded::stale(
                    self.last_valid.clone(),
                    format!("atalhos.toml inválido, mostrando a última versão válida: {error}"),
                )
            }
        }
    }

    fn editable(&mut self, path: &Path) -> Result<ShortcutsFile> {
        let loaded = self.load(path);
        match loaded.problem {
            Some(problem) => Err(ShortcutError::Invalid(problem)),
            None => Ok(ShortcutsFile {
                collapsed: loaded.collapsed,
                items: loaded.shortcuts,
            }),
        }
    }

    fn commit(&mut self, path: &Path, file: ShortcutsFile) -> Result<()> {
        save(path, &file)?;
        self.last_valid = Some(file);
        Ok(())
    }

    pub fn add(&mut self, path: &Path, shortcut: Shortcut) -> Result<usize> {
        let shortcut = validate(shortcut)?;
        let mut file = self.editable(path)?;
        file.items.push(shortcut);
        let index = file.items.len() - 1;
        self.commit(path, file)?;
        Ok(index)
    }

    pub fn update(&mut self, path: &Path, index: usize, shortcut: Shortcut) -> Result<()> {
        let shortcut = validate(shortcut)?;
        let mut file = self.editable(path)?;
        let slot = file
            .items
            .get_mut(index)
            .ok_or(ShortcutError::NotFound(index))?;
        *slot = shortcut;
        self.commit(path, file)
    }

    pub fn remove(&mut self, path: &Path, index: usize) -> Result<Shortcut> {
        let mut file = self.editable(path)?;
        if index >= file.items.len() {
            return Err(ShortcutError::NotFound(index));
        }
        let removed = file.items.remove(index);
        self.commit(path, file)?;
        Ok(removed)
    }

    pub fn move_shortcut(&mut self, path: &Path, from: usize, target: MoveTarget) -> Result<()> {
        let mut file = self.editable(path)?;
        if from >= file.items.len() {
            return Err(ShortcutError::NotFound(from));
        }
        if move_shortcut(&mut file.items, from, &target) {
            self.commit(path, file)?;
        }
        Ok(())
    }

    pub fn move_group(&mut self, path: &Path, group: &str, target: &str) -> Result<()> {
        let mut file = self.editable(path)?;
        if move_group(&mut file.items, group, target) {
            self.commit(path, file)?;
        }
        Ok(())
    }

    pub fn set_collapsed(&mut self, path: &Path, group: &str, collapsed: bool) -> Result<()> {
        let mut file = self.editable(path)?;
        let present = file.collapsed.iter().any(|g| g == group);
        if present == collapsed {
            return Ok(());
        }
        if collapsed {
            file.collapsed.push(group.to_string());
        } else {
            file.collapsed.retain(|g| g != group);
        }
        self.commit(path, file)
    }
}

pub fn move_shortcut(items: &mut Vec<Shortcut>, from: usize, target: &MoveTarget) -> bool {
    if from >= items.len() {
        return false;
    }
    match target {
        MoveTarget::Shortcut(to) => {
            let to = *to;
            if to == from || to >= items.len() {
                return false;
            }
            let group = items[to].group.clone();
            let after = from < to && items[from].group == group;
            let mut item = items.remove(from);
            item.group = group;
            let to = if from < to { to - 1 } else { to };
            items.insert(if after { to + 1 } else { to }, item);
            true
        }
        MoveTarget::GroupEnd(group) => {
            if items.iter().rposition(|s| &s.group == group) == Some(from) {
                return false;
            }
            let mut item = items.remove(from);
            item.group = group.clone();
            match items.iter().rposition(|s| &s.group == group) {
                Some(last) => items.insert(last + 1, item),
                None => items.push(item),
            }
            true
        }
    }
}

pub fn move_group(items: &mut Vec<Shortcut>, group: &str, target: &str) -> bool {
    let order = groups(items);
    let (Some(from), Some(to)) = (
        order.iter().position(|g| g == group),
        order.iter().position(|g| g == target),
    ) else {
        return false;
    };
    if from == to {
        return false;
    }
    let (moved, mut rest): (Vec<Shortcut>, Vec<Shortcut>) = std::mem::take(items)
        .into_iter()
        .partition(|s| s.group == group);
    let position = if from < to {
        rest.iter()
            .rposition(|s| s.group == target)
            .map_or(rest.len(), |i| i + 1)
    } else {
        rest.iter().position(|s| s.group == target).unwrap_or(0)
    };
    rest.splice(position..position, moved);
    *items = rest;
    true
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

fn save(path: &Path, file: &ShortcutsFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, toml::to_string_pretty(file)?)?;
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

pub fn local_path(shortcut: &Shortcut, os: Os) -> Option<PathBuf> {
    let target = shortcut.target.for_os(os)?.trim();
    if target.is_empty() {
        return None;
    }
    match shortcut.kind {
        ShortcutKind::Program => Some(PathBuf::from(target)),
        ShortcutKind::Folder => Some(expand_home(target)),
        ShortcutKind::Url | ShortcutKind::Command => None,
    }
}

pub fn reveal(platform: &dyn Platform, shortcut: &Shortcut) -> Result<()> {
    let path = local_path(shortcut, platform.os()).ok_or(ShortcutError::NoTarget)?;
    platform.reveal_path(&path)?;
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

    #[test]
    fn local_path_and_reveal_cover_programs_and_folders() {
        let program = shortcut("a", ShortcutKind::Program, " C:\\app.exe ", "");
        let folder = shortcut("b", ShortcutKind::Folder, "C:\\Pasta", "");
        let url = shortcut("c", ShortcutKind::Url, "exemplo.com", "");
        assert_eq!(
            local_path(&program, Os::Windows),
            Some(PathBuf::from("C:\\app.exe"))
        );
        assert_eq!(
            local_path(&folder, Os::Windows),
            Some(PathBuf::from("C:\\Pasta"))
        );
        assert_eq!(local_path(&url, Os::Windows), None);
        let platform = FakePlatform::new(Os::Windows);
        reveal(&platform, &folder).unwrap();
        assert!(matches!(
            reveal(&platform, &url),
            Err(ShortcutError::NoTarget)
        ));
        assert_eq!(
            platform.calls(),
            vec![PlatformCall::RevealPath(PathBuf::from("C:\\Pasta"))]
        );
    }

    fn names(items: &[Shortcut]) -> Vec<String> {
        items
            .iter()
            .map(|s| format!("{}:{}", s.group, s.name))
            .collect()
    }

    fn sample() -> Vec<Shortcut> {
        vec![
            shortcut("a", ShortcutKind::Url, "a", "G1"),
            shortcut("b", ShortcutKind::Url, "b", "G1"),
            shortcut("c", ShortcutKind::Url, "c", "G1"),
            shortcut("x", ShortcutKind::Url, "x", "G2"),
            shortcut("y", ShortcutKind::Url, "y", "G2"),
            shortcut("z", ShortcutKind::Url, "z", "G3"),
        ]
    }

    #[test]
    fn move_shortcut_within_and_across_groups() {
        let mut items = sample();
        assert!(move_shortcut(&mut items, 0, &MoveTarget::Shortcut(2)));
        assert_eq!(names(&items)[..3], ["G1:b", "G1:c", "G1:a"]);
        assert!(move_shortcut(&mut items, 2, &MoveTarget::Shortcut(0)));
        assert_eq!(names(&items)[..3], ["G1:a", "G1:b", "G1:c"]);
        assert!(move_shortcut(&mut items, 0, &MoveTarget::Shortcut(4)));
        assert_eq!(
            names(&items),
            ["G1:b", "G1:c", "G2:x", "G2:a", "G2:y", "G3:z"]
        );
        assert!(move_shortcut(
            &mut items,
            5,
            &MoveTarget::GroupEnd("G1".into())
        ));
        assert_eq!(
            names(&items),
            ["G1:b", "G1:c", "G1:z", "G2:x", "G2:a", "G2:y"]
        );
        assert!(!move_shortcut(&mut items, 1, &MoveTarget::Shortcut(1)));
        assert!(!move_shortcut(
            &mut items,
            2,
            &MoveTarget::GroupEnd("G1".into())
        ));
        assert!(!move_shortcut(&mut items, 9, &MoveTarget::Shortcut(0)));
    }

    #[test]
    fn move_group_places_block_around_target() {
        let mut items = sample();
        assert!(move_group(&mut items, "G1", "G2"));
        assert_eq!(groups(&items), ["G2", "G1", "G3"]);
        assert!(move_group(&mut items, "G3", "G2"));
        assert_eq!(groups(&items), ["G3", "G2", "G1"]);
        assert_eq!(names(&items)[1..3], ["G2:x", "G2:y"]);
        assert!(!move_group(&mut items, "G3", "G3"));
        assert!(!move_group(&mut items, "G9", "G1"));
    }

    #[test]
    fn store_persists_order_and_collapsed_groups() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("atalhos.toml");
        let mut store = ShortcutStore::default();
        for item in sample() {
            store.add(&path, item).unwrap();
        }
        store.set_collapsed(&path, "G2", true).unwrap();
        store.set_collapsed(&path, "G2", true).unwrap();
        store
            .move_shortcut(&path, 5, MoveTarget::Shortcut(0))
            .unwrap();
        store.move_group(&path, "G2", "G1").unwrap();
        let loaded = ShortcutStore::default().load(&path);
        assert_eq!(loaded.collapsed, ["G2"]);
        assert_eq!(
            names(&loaded.shortcuts),
            ["G2:x", "G2:y", "G1:z", "G1:a", "G1:b", "G1:c"]
        );
        store.set_collapsed(&path, "G2", false).unwrap();
        assert!(ShortcutStore::default().load(&path).collapsed.is_empty());
        assert!(matches!(
            store.move_shortcut(&path, 42, MoveTarget::Shortcut(0)),
            Err(ShortcutError::NotFound(42))
        ));
    }
}
