pub mod desktop_entry;
mod fake;
pub mod ipc;
mod notify;
pub mod script;
pub mod terminal;
mod tray;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("o hub suporta apenas Windows e Linux");

use std::ffi::{OsStr, OsString};
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
pub enum FileIcon {
    Rgba {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    Encoded(Vec<u8>),
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
    fn file_icon(&self, _target: &Path) -> Option<FileIcon> {
        None
    }
    fn reveal_path(&self, _target: &Path) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("mostrar o arquivo na pasta"))
    }
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

pub fn fold_text(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

pub fn filter_apps<'a>(apps: &'a [InstalledApp], query: &str) -> Vec<(usize, &'a InstalledApp)> {
    let query = fold_text(query);
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(bool, usize, &InstalledApp)> = apps
        .iter()
        .enumerate()
        .filter_map(|(index, app)| {
            let name = fold_text(&app.name);
            terms.iter().all(|term| name.contains(term)).then(|| {
                (
                    terms.first().is_some_and(|t| name.starts_with(t)),
                    index,
                    app,
                )
            })
        })
        .collect();
    found.sort_by_key(|(prefix, _, _)| !prefix);
    found
        .into_iter()
        .map(|(_, index, app)| (index, app))
        .collect()
}

pub fn find_in_path(name: &str, path_var: &OsStr, extensions: &[String]) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let has_extension = Path::new(name).extension().is_some();
    for dir in std::env::split_paths(path_var) {
        let plain = dir.join(name);
        if (has_extension || extensions.is_empty()) && plain.is_file() {
            return Some(plain);
        }
        for extension in extensions {
            let candidate = dir.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn bgra_to_rgba(pixels: &mut [u8]) {
    let opaque = pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0);
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        if opaque {
            pixel[3] = 255;
        } else if pixel[3] > 0 && pixel[3] < 255 {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 / alpha).min(255) as u8;
            }
        }
    }
}

pub fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~:".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> InstalledApp {
        InstalledApp {
            name: name.into(),
            target: format!("{name}.lnk"),
        }
    }

    #[test]
    fn fold_text_ignores_case_and_accents() {
        assert_eq!(
            fold_text("Calculadora CIENTÍFICA Ção"),
            "calculadora cientifica cao"
        );
    }

    #[test]
    fn filter_apps_matches_every_term_and_prefers_prefix() {
        let apps = vec![
            app("Visual Studio Code"),
            app("Código Aberto"),
            app("Steam"),
            app("Microsoft Edge"),
        ];
        let found: Vec<&str> = filter_apps(&apps, "COD")
            .into_iter()
            .map(|(_, a)| a.name.as_str())
            .collect();
        assert_eq!(found, vec!["Código Aberto", "Visual Studio Code"]);
        let found = filter_apps(&apps, "studio vis");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 0);
        assert_eq!(filter_apps(&apps, "  ").len(), 4);
        assert!(filter_apps(&apps, "firefox").is_empty());
    }

    #[test]
    fn find_in_path_uses_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("tool.exe"), "").unwrap();
        std::fs::write(bin.join("plain"), "").unwrap();
        let path_var = std::env::join_paths([temp.path().join("vazio"), bin.clone()]).unwrap();
        let exts = vec![".com".to_string(), ".exe".to_string()];
        assert_eq!(
            find_in_path("tool", &path_var, &exts),
            Some(bin.join("tool.exe"))
        );
        assert_eq!(
            find_in_path("tool.exe", &path_var, &exts),
            Some(bin.join("tool.exe"))
        );
        assert_eq!(
            find_in_path("plain", &path_var, &[]),
            Some(bin.join("plain"))
        );
        assert_eq!(find_in_path("plain", &path_var, &exts), None);
        assert_eq!(find_in_path("bin/tool", &path_var, &exts), None);
    }

    #[test]
    fn bgra_is_converted_and_unpremultiplied() {
        let mut pixels = vec![10, 20, 30, 255, 0, 64, 128, 128];
        bgra_to_rgba(&mut pixels);
        assert_eq!(pixels, vec![30, 20, 10, 255, 255, 127, 0, 128]);
        let mut no_alpha = vec![1, 2, 3, 0];
        bgra_to_rgba(&mut no_alpha);
        assert_eq!(no_alpha, vec![3, 2, 1, 255]);
    }

    #[test]
    fn file_uri_escapes_special_characters() {
        assert_eq!(
            file_uri(Path::new("/home/eu/Meus Arquivos/ação.txt")),
            "file:///home/eu/Meus%20Arquivos/a%C3%A7%C3%A3o.txt"
        );
        assert_eq!(
            file_uri(Path::new("C:\\Pasta\\a b")),
            "file:///C:/Pasta/a%20b"
        );
    }

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
