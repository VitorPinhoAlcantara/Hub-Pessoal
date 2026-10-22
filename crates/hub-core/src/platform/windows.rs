use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetDesktopWindow, GetForegroundWindow, GetMessageW, GetShellWindow,
    GetWindowRect, MSG, SW_SHOWNORMAL, TranslateMessage,
};

use super::{
    ActionHandler, FileIcon, InstalledApp, MediaSession, Notification, Os, Platform, PlatformError,
    TrayHandler, TrayImage, bgra_to_rgba, collect_files, dedup_apps, find_in_path,
};

const ICON_PIXELS: i32 = 48;

pub struct WindowsPlatform;

struct WindowsMedia;

fn media_error(error: windows::core::Error) -> PlatformError {
    PlatformError::Launch {
        target: "mídia do sistema".to_string(),
        reason: error.message(),
    }
}

fn current_media_session()
-> Result<Option<windows::Media::Control::GlobalSystemMediaTransportControlsSession>, PlatformError>
{
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;
    static MANAGER: std::sync::OnceLock<Manager> = std::sync::OnceLock::new();
    let manager = match MANAGER.get() {
        Some(manager) => manager,
        None => {
            let manager = Manager::RequestAsync()
                .and_then(|operation| operation.join())
                .map_err(media_error)?;
            MANAGER.get_or_init(|| manager)
        }
    };
    Ok(manager.GetCurrentSession().ok())
}

impl MediaSession for WindowsMedia {
    fn current(&self) -> Result<Option<super::MediaInfo>, PlatformError> {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus;
        let Some(session) = current_media_session()? else {
            return Ok(None);
        };
        let properties = session
            .TryGetMediaPropertiesAsync()
            .and_then(|operation| operation.join())
            .map_err(media_error)?;
        let title = properties
            .Title()
            .map(|t| t.to_string_lossy())
            .unwrap_or_default();
        let artist = properties
            .Artist()
            .map(|a| a.to_string_lossy())
            .ok()
            .filter(|a| !a.is_empty());
        let playing = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|status| status == PlaybackStatus::Playing)
            .unwrap_or(false);
        if title.is_empty() && artist.is_none() {
            return Ok(None);
        }
        Ok(Some(super::MediaInfo {
            title,
            artist,
            playing,
        }))
    }

    fn send(&self, command: super::MediaCommand) -> Result<(), PlatformError> {
        let Some(session) = current_media_session()? else {
            return Err(PlatformError::Unsupported("nenhuma mídia tocando"));
        };
        let operation = match command {
            super::MediaCommand::PlayPause => session.TryTogglePlayPauseAsync(),
            super::MediaCommand::Next => session.TrySkipNextAsync(),
            super::MediaCommand::Previous => session.TrySkipPreviousAsync(),
        };
        operation
            .and_then(|operation| operation.join())
            .map(|_| ())
            .map_err(media_error)
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn shell_open(target: &OsStr) -> Result<(), PlatformError> {
    let operation = wide(OsStr::new("open"));
    let file = wide(target);
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(PlatformError::Launch {
            target: target.to_string_lossy().into_owned(),
            reason: format!("ShellExecute retornou {}", result as isize),
        })
    }
}

fn run_tray_loop() {
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let suffix = Path::new("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");
    ["APPDATA", "ProgramData"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|base| PathBuf::from(base).join(&suffix))
        .collect()
}

fn is_uninstaller(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("uninstall") || lower.contains("desinstalar")
}

fn path_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn resolve_target(target: &Path) -> Option<PathBuf> {
    if target.exists() {
        return Some(target.to_path_buf());
    }
    let name = target.to_str()?;
    let path_var = std::env::var_os("PATH")?;
    find_in_path(name, &path_var, &path_extensions())
}

struct ComScope(bool);

impl ComScope {
    fn enter() -> Self {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

fn shell_icon(path: &Path) -> Option<FileIcon> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
    };
    use windows::core::PCWSTR;
    let _com = ComScope::enter();
    let name = wide(path.as_os_str());
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None) }.ok()?;
    let size = SIZE {
        cx: ICON_PIXELS,
        cy: ICON_PIXELS,
    };
    let bitmap = unsafe { factory.GetImage(size, SIIGBF_ICONONLY) }.ok()?;
    let icon = bitmap_pixels(bitmap);
    unsafe {
        let _ = DeleteObject(bitmap.into());
    }
    icon
}

fn bitmap_pixels(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<FileIcon> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        GetDIBits, GetObjectW,
    };
    let mut info = BITMAP::default();
    let read = unsafe {
        GetObjectW(
            bitmap.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut info as *mut BITMAP).cast()),
        )
    };
    if read == 0 || info.bmWidth <= 0 || info.bmHeight == 0 {
        return None;
    }
    let width = info.bmWidth;
    let height = info.bmHeight.abs();
    let mut header = BITMAPINFO::default();
    header.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    header.bmiHeader.biWidth = width;
    header.bmiHeader.biHeight = -height;
    header.bmiHeader.biPlanes = 1;
    header.bmiHeader.biBitCount = 32;
    header.bmiHeader.biCompression = BI_RGB.0;
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        lines
    };
    if lines <= 0 {
        return None;
    }
    bgra_to_rgba(&mut pixels);
    Some(FileIcon::Rgba {
        width: width as u32,
        height: height as u32,
        pixels,
    })
}

impl Platform for WindowsPlatform {
    fn os(&self) -> Os {
        Os::Windows
    }

    fn open_program(&self, target: &str) -> Result<(), PlatformError> {
        shell_open(OsStr::new(target))
    }

    fn open_path(&self, path: &Path) -> Result<(), PlatformError> {
        shell_open(path.as_os_str())
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        shell_open(OsStr::new(url))
    }

    fn list_installed_apps(&self) -> Result<Vec<InstalledApp>, PlatformError> {
        let apps = start_menu_dirs()
            .iter()
            .flat_map(|dir| collect_files(dir, "lnk"))
            .filter_map(|path| {
                let name = path.file_stem()?.to_string_lossy().into_owned();
                (!is_uninstaller(&name)).then(|| InstalledApp {
                    name,
                    target: path.to_string_lossy().into_owned(),
                })
            })
            .collect();
        Ok(dedup_apps(apps))
    }

    fn file_icon(&self, target: &Path) -> Option<FileIcon> {
        shell_icon(&resolve_target(target)?)
    }

    fn reveal_path(&self, target: &Path) -> Result<(), PlatformError> {
        use std::os::windows::process::CommandExt;
        let path = resolve_target(target).ok_or_else(|| PlatformError::Launch {
            target: target.to_string_lossy().into_owned(),
            reason: "arquivo não encontrado".to_string(),
        })?;
        let child = std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map_err(|error| PlatformError::Launch {
                target: path.to_string_lossy().into_owned(),
                reason: error.to_string(),
            })?;
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        Ok(())
    }

    fn run_command(&self, command: &str) -> Result<(), PlatformError> {
        use std::os::windows::process::CommandExt;
        let spec = super::shell_command(Os::Windows, command);
        let mut process = spec.to_command();
        process
            .creation_flags(0x0800_0000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = process.spawn().map_err(|error| PlatformError::Launch {
            target: command.to_string(),
            reason: error.to_string(),
        })?;
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        Ok(())
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
        Ok(Box::new(WindowsMedia))
    }

    fn is_fullscreen_app_running(&self) -> bool {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() || window == GetDesktopWindow() || window == GetShellWindow() {
                return false;
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(window, &mut rect) == 0 {
                return false;
            }
            let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return false;
            }
            let screen = info.rcMonitor;
            rect.left <= screen.left
                && rect.top <= screen.top
                && rect.right >= screen.right
                && rect.bottom >= screen.bottom
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstallers_are_filtered() {
        assert!(is_uninstaller("Uninstall Foo"));
        assert!(is_uninstaller("Desinstalar Bar"));
        assert!(!is_uninstaller("Visual Studio Code"));
    }

    #[test]
    fn start_menu_dirs_point_to_programs() {
        for dir in start_menu_dirs() {
            assert!(dir.ends_with("Programs"));
        }
    }

    #[test]
    fn commands_are_resolved_through_path() {
        let resolved = resolve_target(Path::new("cmd")).unwrap();
        assert!(
            resolved
                .to_string_lossy()
                .to_lowercase()
                .ends_with("cmd.exe")
        );
        assert!(resolve_target(Path::new("programa-que-nao-existe-123")).is_none());
    }

    #[test]
    fn system_executable_has_icon() {
        let root = std::env::var_os("SystemRoot").unwrap();
        let explorer = PathBuf::from(root).join("explorer.exe");
        let Some(FileIcon::Rgba {
            width,
            height,
            pixels,
        }) = WindowsPlatform.file_icon(&explorer)
        else {
            panic!("esperava o ícone do explorer");
        };
        assert!(width > 0 && height > 0);
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    }

    #[test]
    fn missing_file_has_no_icon() {
        assert!(
            WindowsPlatform
                .file_icon(Path::new("Z:\\nada\\aqui.exe"))
                .is_none()
        );
    }

    #[test]
    fn listing_installed_apps_does_not_fail() {
        assert!(WindowsPlatform.list_installed_apps().is_ok());
    }
}
