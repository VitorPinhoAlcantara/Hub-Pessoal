use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::desktop_entry::{self, DesktopEntry};
use super::{InstalledApp, MediaSession, Os, Platform, PlatformError, collect_files, dedup_apps};

pub struct LinuxPlatform;

fn spawn_detached(program: &str, args: &[String]) -> Result<(), PlatformError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| PlatformError::Launch {
            target: program.to_string(),
            reason: error.to_string(),
        })?;
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

fn applications_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".local").join("share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    data_home
        .into_iter()
        .chain(data_dirs.split(':').map(PathBuf::from))
        .map(|dir| dir.join("applications"))
        .collect()
}

fn desktop_id(root: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("-"))
}

fn desktop_entries() -> Vec<(String, DesktopEntry)> {
    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for root in applications_dirs() {
        for file in collect_files(&root, "desktop") {
            let Some(id) = desktop_id(&root, &file) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some(entry) = desktop_entry::parse(&content) {
                entries.push((id, entry));
            }
        }
    }
    entries
}

fn launch_exec(exec: &str, target: &str) -> Result<(), PlatformError> {
    let args = desktop_entry::exec_to_args(exec);
    let Some((program, rest)) = args.split_first() else {
        return Err(PlatformError::Launch {
            target: target.to_string(),
            reason: "linha Exec vazia".to_string(),
        });
    };
    spawn_detached(program, rest)
}

impl Platform for LinuxPlatform {
    fn os(&self) -> Os {
        Os::Linux
    }

    fn open_program(&self, target: &str) -> Result<(), PlatformError> {
        if target.ends_with(".desktop") {
            let path = Path::new(target);
            if path.is_file() {
                let content = std::fs::read_to_string(path)?;
                if let Some(entry) = desktop_entry::parse(&content) {
                    return launch_exec(&entry.exec, target);
                }
            }
            if let Some((_, entry)) = desktop_entries().into_iter().find(|(id, _)| id == target) {
                return launch_exec(&entry.exec, target);
            }
            return Err(PlatformError::Launch {
                target: target.to_string(),
                reason: "arquivo .desktop não encontrado".to_string(),
            });
        }
        if Path::new(target).exists() {
            return self.open_path(Path::new(target));
        }
        launch_exec(target, target)
    }

    fn open_path(&self, path: &Path) -> Result<(), PlatformError> {
        spawn_detached("xdg-open", &[path.to_string_lossy().into_owned()])
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        spawn_detached("xdg-open", &[url.to_string()])
    }

    fn list_installed_apps(&self) -> Result<Vec<InstalledApp>, PlatformError> {
        let apps = desktop_entries()
            .into_iter()
            .filter(|(_, entry)| entry.visible)
            .map(|(id, entry)| InstalledApp {
                name: entry.name,
                target: id,
            })
            .collect();
        Ok(dedup_apps(apps))
    }

    fn media_session(&self) -> Result<Box<dyn MediaSession>, PlatformError> {
        Err(PlatformError::Unsupported("sessão de mídia"))
    }

    fn is_fullscreen_app_running(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_id_joins_subfolders_with_dash() {
        let root = Path::new("/usr/share/applications");
        let file = root.join("kde4").join("app.desktop");
        assert_eq!(desktop_id(root, &file).unwrap(), "kde4-app.desktop");
    }

    #[test]
    fn applications_dirs_are_not_empty() {
        assert!(!applications_dirs().is_empty());
    }

    #[test]
    fn listing_installed_apps_does_not_fail() {
        assert!(LinuxPlatform.list_installed_apps().is_ok());
    }
}
