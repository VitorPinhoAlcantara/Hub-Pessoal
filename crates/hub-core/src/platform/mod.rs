pub mod desktop_entry;
mod fake;
pub mod ipc;
mod notify;
pub mod script;
mod tray;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("o hub suporta apenas Windows e Linux");

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use fake::{FakePlatform, PlatformCall};
pub use script::script_command_for;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("recurso indisponível nesta plataforma: {0}")]
    Unsupported(&'static str),
    #[error("falha ao abrir {target}: {reason}")]
    Launch { target: String, reason: String },
    #[error("nenhum interpretador conhecido para {0}")]
    NoInterpreter(PathBuf),
    #[error("erro de entrada e saída: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        #[cfg(windows)]
        {
            Os::Windows
        }
        #[cfg(not(windows))]
        {
            Os::Linux
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InstalledApp {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl CommandSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn to_command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args);
        command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: Option<String>,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
}

pub type ActionHandler = Box<dyn FnOnce(Option<String>) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    Quit,
}

pub type TrayHandler = Box<dyn Fn(TrayEvent) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub trait MediaSession: Send + Sync {
    fn current(&self) -> Result<Option<MediaInfo>, PlatformError>;
    fn send(&self, command: MediaCommand) -> Result<(), PlatformError>;
}

pub trait Platform: Send + Sync {
    fn os(&self) -> Os;
    fn open_program(&self, target: &str) -> Result<(), PlatformError>;
    fn open_path(&self, path: &Path) -> Result<(), PlatformError>;
    fn open_url(&self, url: &str) -> Result<(), PlatformError>;
    fn list_installed_apps(&self) -> Result<Vec<InstalledApp>, PlatformError>;
    fn run_command(&self, command: &str) -> Result<(), PlatformError>;
    fn script_command(&self, script: &Path) -> Result<CommandSpec, PlatformError> {
        script_command_for(script, self.os())
    }
    fn notify(
        &self,
        notification: Notification,
        on_action: Option<ActionHandler>,
    ) -> Result<(), PlatformError>;
    fn spawn_tray(&self, image: TrayImage, handler: TrayHandler) -> Result<(), PlatformError>;
    fn media_session(&self) -> Result<Box<dyn MediaSession>, PlatformError>;
    fn is_fullscreen_app_running(&self) -> bool;
}

fn spawn_tray_thread(
    image: TrayImage,
    handler: TrayHandler,
    run_loop: fn(),
) -> Result<(), PlatformError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("hub-bandeja".to_string())
        .spawn(move || match tray::build(image, "hub", handler) {
            Ok(icon) => {
                let _ = ready_tx.send(Ok(()));
                run_loop();
                drop(icon);
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })?;
    ready_rx.recv().unwrap_or_else(|_| {
        Err(PlatformError::Launch {
            target: "bandeja".to_string(),
            reason: "a thread da bandeja terminou sem responder".to_string(),
        })
    })
}

pub fn hide_console(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

pub fn hide_console_std(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

pub fn shell_command(os: Os, command: &str) -> CommandSpec {
    match os {
        Os::Windows => CommandSpec::new("cmd").arg("/C").arg(command),
        Os::Linux => CommandSpec::new("sh").arg("-c").arg(command),
    }
}

pub fn preferred_gpu_backend() -> Option<&'static str> {
    #[cfg(windows)]
    {
        Some("dx12")
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn native() -> Arc<dyn Platform> {
    #[cfg(windows)]
    {
        Arc::new(windows::WindowsPlatform)
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxPlatform)
    }
}

pub fn collect_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

pub fn dedup_apps(mut apps: Vec<InstalledApp>) -> Vec<InstalledApp> {
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_files_walks_subfolders_and_filters_extension() {
        let temp = tempfile::tempdir().unwrap();
        let sub = temp.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(temp.path().join("um.lnk"), "").unwrap();
        std::fs::write(sub.join("dois.LNK"), "").unwrap();
        std::fs::write(sub.join("tres.txt"), "").unwrap();
        let files = collect_files(temp.path(), "lnk");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn collect_files_ignores_missing_folder() {
        assert!(collect_files(Path::new("pasta-que-nao-existe"), "lnk").is_empty());
    }

    #[test]
    fn dedup_apps_sorts_and_removes_repeated_names() {
        let apps = vec![
            InstalledApp {
                name: "Zed".into(),
                target: "1".into(),
            },
            InstalledApp {
                name: "code".into(),
                target: "2".into(),
            },
            InstalledApp {
                name: "Code".into(),
                target: "3".into(),
            },
        ];
        let result = dedup_apps(apps);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].target, "2");
        assert_eq!(result[1].name, "Zed");
    }

    #[test]
    fn native_platform_matches_current_os() {
        assert_eq!(native().os(), Os::current());
    }
}
