use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::desktop_entry::{self, DesktopEntry};
use super::{
    ActionHandler, FileIcon, InstalledApp, MediaSession, Notification, Os, Platform, PlatformError,
    TrayHandler, TrayImage, TrayMenuState, collect_files, dedup_apps, file_uri, find_in_path,
};

pub struct LinuxPlatform;

struct LinuxMedia;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";

fn media_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Launch {
        target: "mídia do sistema".to_string(),
        reason: error.to_string(),
    }
}

fn mpris_player(
    connection: &zbus::blocking::Connection,
) -> Result<Option<zbus::blocking::Proxy<'static>>, PlatformError> {
    let dbus = zbus::blocking::fdo::DBusProxy::new(connection).map_err(media_error)?;
    let names: Vec<String> = dbus
        .list_names()
        .map_err(media_error)?
        .into_iter()
        .map(|name| name.to_string())
        .filter(|name| name.starts_with(MPRIS_PREFIX))
        .collect();
    let mut fallback = None;
    for name in names {
        let proxy =
            zbus::blocking::Proxy::new_owned(connection.clone(), name, MPRIS_PATH, MPRIS_PLAYER)
                .map_err(media_error)?;
        let status: String = proxy.get_property("PlaybackStatus").unwrap_or_default();
        if status == "Playing" {
            return Ok(Some(proxy));
        }
        if fallback.is_none() {
            fallback = Some(proxy);
        }
    }
    Ok(fallback)
}

impl MediaSession for LinuxMedia {
    fn current(&self) -> Result<Option<super::MediaInfo>, PlatformError> {
        let connection = zbus::blocking::Connection::session().map_err(media_error)?;
        let Some(player) = mpris_player(&connection)? else {
            return Ok(None);
        };
        let status: String = player.get_property("PlaybackStatus").unwrap_or_default();
        let mut metadata: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
            player.get_property("Metadata").map_err(media_error)?;
        let title = metadata
            .remove("xesam:title")
            .and_then(|value| String::try_from(value).ok())
            .unwrap_or_default();
        let artist = metadata
            .remove("xesam:artist")
            .and_then(|value| Vec::<String>::try_from(value).ok())
            .and_then(|artists| artists.into_iter().next())
            .filter(|artist| !artist.is_empty());
        if title.is_empty() && artist.is_none() {
            return Ok(None);
        }
        Ok(Some(super::MediaInfo {
            title,
            artist,
            playing: status == "Playing",
        }))
    }

    fn send(&self, command: super::MediaCommand) -> Result<(), PlatformError> {
        let connection = zbus::blocking::Connection::session().map_err(media_error)?;
        let Some(player) = mpris_player(&connection)? else {
            return Err(PlatformError::Unsupported("nenhuma mídia tocando"));
        };
        let method = match command {
            super::MediaCommand::PlayPause => "PlayPause",
            super::MediaCommand::Next => "Next",
            super::MediaCommand::Previous => "Previous",
        };
        player.call_method(method, &()).map_err(media_error)?;
        Ok(())
    }
}

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

fn run_tray_loop(apply: &mut dyn FnMut()) {
    loop {
        apply();
        std::thread::park_timeout(std::time::Duration::from_millis(500));
    }
}

pub(super) fn set_autostart(exe: &Path, enabled: bool) -> Result<(), PlatformError> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or(PlatformError::Unsupported("pasta de configuração do usuário"))?;
    let dir = config_home.join("autostart");
    let file = dir.join(format!("{}.desktop", crate::channel::Channel::current().dir_name()));
    if !enabled {
        return match std::fs::remove_file(&file) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        file,
        format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\" --bandeja\nX-GNOME-Autostart-enabled=true\n",
            crate::channel::app_name(),
            exe.display()
        ),
    )?;
    Ok(())
}

fn data_dirs() -> Vec<PathBuf> {
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
        .collect()
}

fn icon_file(icon: &str) -> Option<PathBuf> {
    let dirs = data_dirs();
    let mut icon_roots: Vec<PathBuf> = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".icons"))
        .into_iter()
        .collect();
    icon_roots.extend(dirs.iter().map(|dir| dir.join("icons")));
    let mut pixmap_roots: Vec<PathBuf> = dirs.iter().map(|dir| dir.join("pixmaps")).collect();
    pixmap_roots.push(PathBuf::from("/usr/share/pixmaps"));
    desktop_entry::icon_candidates(icon, &icon_roots, &pixmap_roots)
        .into_iter()
        .find(|path| path.is_file())
}

fn desktop_file(id: &str) -> Option<PathBuf> {
    applications_dirs().into_iter().find_map(|root| {
        collect_files(&root, "desktop")
            .into_iter()
            .find(|file| desktop_id(&root, file).as_deref() == Some(id))
    })
}

pub(super) fn program_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/snap/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    dirs
}

fn resolve_target(target: &Path) -> Option<PathBuf> {
    if target.exists() {
        return Some(target.to_path_buf());
    }
    let name = target.to_str()?;
    if name.ends_with(".desktop") {
        return desktop_file(name);
    }
    let path_var = std::env::var_os("PATH")?;
    find_in_path(name, &path_var, &[])
}

fn entry_for_program(program: &Path) -> Option<DesktopEntry> {
    let name = program.file_name()?.to_str()?;
    desktop_entries()
        .into_iter()
        .map(|(_, entry)| entry)
        .find(|entry| {
            desktop_entry::exec_to_args(&entry.exec)
                .first()
                .and_then(|first| Path::new(first).file_name().map(|f| f == name))
                .unwrap_or(false)
        })
}

fn desktop_icon(path: &Path) -> Option<FileIcon> {
    let entry = if path.extension().is_some_and(|ext| ext == "desktop") {
        desktop_entry::parse(&std::fs::read_to_string(path).ok()?)?
    } else if path.is_file() {
        entry_for_program(path)?
    } else {
        return None;
    };
    let file = icon_file(entry.icon.as_deref()?)?;
    std::fs::read(file).ok().map(FileIcon::Encoded)
}

fn show_in_file_manager(path: &Path) -> Result<(), PlatformError> {
    let connection = zbus::blocking::Connection::session().map_err(media_error)?;
    connection
        .call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![file_uri(path)], ""),
        )
        .map_err(media_error)?;
    Ok(())
}

fn applications_dirs() -> Vec<PathBuf> {
    data_dirs()
        .into_iter()
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

    fn file_icon(&self, target: &Path) -> Option<FileIcon> {
        desktop_icon(&resolve_target(target)?)
    }

    fn reveal_path(&self, target: &Path) -> Result<(), PlatformError> {
        let path = resolve_target(target).ok_or_else(|| PlatformError::Launch {
            target: target.to_string_lossy().into_owned(),
            reason: "arquivo não encontrado".to_string(),
        })?;
        if show_in_file_manager(&path).is_ok() {
            return Ok(());
        }
        let folder = path.parent().unwrap_or(&path).to_path_buf();
        self.open_path(&folder)
    }

    fn run_command(&self, command: &str) -> Result<(), PlatformError> {
        spawn_detached("sh", &["-c".to_string(), command.to_string()])
    }

    fn notify(
        &self,
        notification: Notification,
        on_action: Option<ActionHandler>,
    ) -> Result<(), PlatformError> {
        super::notify::show(notification, on_action)
    }

    fn spawn_tray(&self, image: TrayImage, handler: TrayHandler) -> Result<(), PlatformError> {
        super::spawn_tray_thread(image, handler, run_tray_loop)
    }

    fn update_tray(&self, state: TrayMenuState) {
        super::tray::queue(state);
    }

    fn media_session(&self) -> Result<Box<dyn MediaSession>, PlatformError> {
        Ok(Box::new(LinuxMedia))
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
    fn existing_paths_resolve_to_themselves() {
        let temp = std::env::temp_dir();
        assert_eq!(resolve_target(&temp), Some(temp.clone()));
        assert!(resolve_target(Path::new("programa-que-nao-existe-123")).is_none());
    }

    #[test]
    fn missing_target_has_no_icon() {
        assert!(LinuxPlatform.file_icon(Path::new("/nada/aqui")).is_none());
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
