pub mod clipboard;
pub mod desktop_entry;
mod fake;
pub mod gpu;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    Quit,
    Mute(u32),
    Unmute,
    RunScript(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayMenuState {
    pub scripts: Vec<(String, String)>,
    pub muted_until: Option<String>,
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
    fn update_tray(&self, _state: TrayMenuState) {}
    fn media_session(&self) -> Result<Box<dyn MediaSession>, PlatformError>;
    fn is_fullscreen_app_running(&self) -> bool;
}

fn spawn_tray_thread(
    image: TrayImage,
    handler: TrayHandler,
    run_loop: fn(&mut dyn FnMut()),
) -> Result<(), PlatformError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("hub-bandeja".to_string())
        .spawn(
            move || match tray::build(image, crate::channel::app_name(), handler) {
                Ok(mut controller) => {
                    let _ = ready_tx.send(Ok(()));
                    run_loop(&mut || controller.apply_pending());
                    drop(controller);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            },
        )?;
    ready_rx.recv().unwrap_or_else(|_| {
        Err(PlatformError::Launch {
            target: "bandeja".to_string(),
            reason: "a thread da bandeja terminou sem responder".to_string(),
        })
    })
}

pub fn npm_cli(name: &str) -> CommandSpec {
    #[cfg(windows)]
    {
        CommandSpec::new("cmd").arg("/C").arg(format!("{name}.cmd"))
    }
    #[cfg(not(windows))]
    {
        CommandSpec::new(name)
    }
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

pub fn spawn_detached(program: &Path, args: &[&str]) -> std::io::Result<()> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    hide_console_std(&mut command);
    command.spawn().map(|_| ())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowDrag {
    Move,
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

pub fn begin_window_drag(raw_window: u64, drag: WindowDrag) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows::begin_window_drag(raw_window, drag);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (raw_window, drag);
        Err(PlatformError::Unsupported("arrastar a janela pelo sistema"))
    }
}

pub fn ytdlp_cookie_source(browser: crate::config::BrowserLogin) -> Option<String> {
    use crate::config::BrowserLogin;
    match browser {
        BrowserLogin::None => None,
        BrowserLogin::Zen => {
            firefox_family_profile(&zen_root()?).map(|p| format!("firefox:{}", p.display()))
        }
        BrowserLogin::Firefox => Some(
            firefox_root()
                .and_then(|root| firefox_family_profile(&root))
                .map(|p| format!("firefox:{}", p.display()))
                .unwrap_or_else(|| "firefox".to_string()),
        ),
        BrowserLogin::Chrome => Some("chrome".to_string()),
        BrowserLogin::Edge => Some("edge".to_string()),
        BrowserLogin::Brave => Some("brave".to_string()),
    }
}

fn zen_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("zen"))
    }
    #[cfg(not(windows))]
    {
        directories::UserDirs::new().map(|dirs| dirs.home_dir().join(".zen"))
    }
}

fn firefox_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Mozilla").join("Firefox"))
    }
    #[cfg(not(windows))]
    {
        directories::UserDirs::new().map(|dirs| dirs.home_dir().join(".mozilla").join("firefox"))
    }
}

pub fn firefox_family_profile(root: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(root.join("profiles.ini")).ok()?;
    let mut install_default = None;
    let mut marked_default = None;
    let mut first = None;
    let mut section = String::new();
    let mut path = None;
    let mut relative = true;
    let mut is_default = false;
    let mut finish = |section: &str, path: Option<String>, relative: bool, is_default: bool| {
        let Some(path) = path else {
            return;
        };
        let full = if relative {
            root.join(&path)
        } else {
            PathBuf::from(&path)
        };
        if section.starts_with("Install") {
            install_default.get_or_insert(full);
        } else if section.starts_with("Profile") {
            if is_default {
                marked_default.get_or_insert(full.clone());
            }
            first.get_or_insert(full);
        }
    };
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            finish(&section, path.take(), relative, is_default);
            section = name.to_string();
            relative = true;
            is_default = false;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Path" => path = Some(value.trim().to_string()),
            "Default" if section.starts_with("Install") => path = Some(value.trim().to_string()),
            "Default" => is_default = value.trim() == "1",
            "IsRelative" => relative = value.trim() != "0",
            _ => {}
        }
    }
    finish(&section, path.take(), relative, is_default);
    install_default
        .or(marked_default)
        .or(first)
        .filter(|profile| profile.join("cookies.sqlite").exists())
}

pub fn set_autostart(enabled: bool) -> Result<(), PlatformError> {
    let exe = std::env::current_exe()?;
    #[cfg(windows)]
    {
        windows::set_autostart(&exe, enabled)
    }
    #[cfg(not(windows))]
    {
        linux::set_autostart(&exe, enabled)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FocusState {
    pub game_mode: Option<u32>,
    pub quiet_hours: Option<u32>,
}

pub fn enter_game_mode() -> FocusState {
    #[cfg(windows)]
    {
        windows::enter_game_mode()
    }
    #[cfg(not(windows))]
    {
        FocusState::default()
    }
}

pub fn leave_game_mode(previous: FocusState) {
    #[cfg(windows)]
    windows::leave_game_mode(previous);
    #[cfg(not(windows))]
    let _ = previous;
}

pub fn transparency_effects_enabled() -> bool {
    #[cfg(windows)]
    {
        windows::transparency_effects_enabled()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

pub fn apply_taskbar_icon(raw_window: u64) {
    #[cfg(windows)]
    windows::apply_taskbar_icon(raw_window);
    #[cfg(not(windows))]
    let _ = raw_window;
}

pub fn desktop_wallpaper() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows::desktop_wallpaper()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn apply_acrylic(raw_window: u64, tint: (u8, u8, u8, u8)) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows::apply_acrylic(raw_window, tint)
    }
    #[cfg(not(windows))]
    {
        let _ = (raw_window, tint);
        Err(PlatformError::Unsupported("efeito acrílico da janela"))
    }
}

pub fn clear_acrylic(raw_window: u64) {
    #[cfg(windows)]
    windows::clear_acrylic(raw_window);
    #[cfg(not(windows))]
    let _ = raw_window;
}

pub fn apply_window_blur(
    raw_window: u64,
    blur: crate::config::WindowBlur,
    tint: (u8, u8, u8, u8),
) -> Result<(), PlatformError> {
    use crate::config::WindowBlur;
    #[cfg(windows)]
    {
        windows::clear_acrylic(raw_window);
        windows::clear_blur(raw_window);
        match blur {
            WindowBlur::None => Ok(()),
            WindowBlur::Light => windows::apply_blur(raw_window, tint),
            WindowBlur::Strong => windows::apply_acrylic(raw_window, tint),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (raw_window, tint);
        match blur {
            WindowBlur::None => Ok(()),
            _ => Err(PlatformError::Unsupported("desfoque da janela")),
        }
    }
}

pub fn launch_installer(installer: &Path, relaunch: &Path) -> Result<(), PlatformError> {
    #[cfg(windows)]
    {
        windows::launch_installer(installer, relaunch)
    }
    #[cfg(not(windows))]
    {
        let _ = (installer, relaunch);
        Err(PlatformError::Unsupported(
            "atualização automática só está disponível no Windows",
        ))
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

pub fn find_program(name: &str) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        vec![".exe".to_string()]
    } else {
        Vec::new()
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    dirs.extend(windows::program_dirs(name));
    #[cfg(target_os = "linux")]
    dirs.extend(linux::program_dirs());
    find_in_dirs(name, dirs, &extensions)
}

pub fn find_in_path(name: &str, path_var: &OsStr, extensions: &[String]) -> Option<PathBuf> {
    find_in_dirs(name, std::env::split_paths(path_var), extensions)
}

pub fn find_in_dirs(
    name: &str,
    dirs: impl IntoIterator<Item = PathBuf>,
    extensions: &[String],
) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let has_extension = Path::new(name).extension().is_some();
    for dir in dirs {
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

pub fn default_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        match find_program("pwsh") {
            Some(path) => (
                path.to_string_lossy().into_owned(),
                vec!["-NoLogo".to_string()],
            ),
            None => ("powershell.exe".to_string(), vec!["-NoLogo".to_string()]),
        }
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/bin/bash".to_string());
        (shell, Vec::new())
    }
}

pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

pub fn terminal_font_family() -> Option<&'static str> {
    #[cfg(windows)]
    {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| OsString::from("C:\\Windows"));
        dirs.push(PathBuf::from(windir).join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
        let candidates = [
            ("Cascadia Mono", "CascadiaMono.ttf"),
            ("Consolas", "consola.ttf"),
        ];
        candidates
            .into_iter()
            .find(|(_, file)| dirs.iter().any(|dir| dir.join(file).is_file()))
            .map(|(family, _)| family)
    }
    #[cfg(not(windows))]
    {
        None
    }
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
    fn firefox_family_profile_prefers_install_default() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for name in ["a.Default Profile", "b.Default (release)"] {
            std::fs::create_dir_all(root.join("Profiles").join(name)).unwrap();
            std::fs::write(root.join("Profiles").join(name).join("cookies.sqlite"), "").unwrap();
        }
        std::fs::write(
            root.join("profiles.ini"),
            "[InstallX]
Default=Profiles/b.Default (release)
Locked=1

[Profile1]
Name=Default Profile
IsRelative=1
Path=Profiles/a.Default Profile
Default=1
",
        )
        .unwrap();
        assert_eq!(
            firefox_family_profile(root),
            Some(root.join("Profiles/b.Default (release)"))
        );
        std::fs::write(
            root.join("profiles.ini"),
            "[Profile0]
IsRelative=1
Path=Profiles/a.Default Profile
Default=1
",
        )
        .unwrap();
        assert_eq!(
            firefox_family_profile(root),
            Some(root.join("Profiles/a.Default Profile"))
        );
        assert_eq!(firefox_family_profile(&root.join("nada")), None);
    }

    #[test]
    fn native_platform_matches_current_os() {
        assert_eq!(native().os(), Os::current());
    }
}
