use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::desktop_entry::{self, DesktopEntry};
use super::{
    ActionHandler, InstalledApp, MediaSession, Notification, Os, Platform, PlatformError,
    TrayHandler, TrayImage, collect_files, dedup_apps,
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

fn run_tray_loop() {
    loop {
        std::thread::park();
    }
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
    fn applications_dirs_are_not_empty() {
        assert!(!applications_dirs().is_empty());
    }

    #[test]
    fn listing_installed_apps_does_not_fail() {
        assert!(LinuxPlatform.list_installed_apps().is_ok());
    }
}
