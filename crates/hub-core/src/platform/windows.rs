use std::ffi::OsStr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetDesktopWindow, GetForegroundWindow, GetMessageW, GetShellWindow,
    GetWindowRect, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SW_SHOWNORMAL,
    TranslateMessage, WM_APP,
};

use super::{
    ActionHandler, FileIcon, InstalledApp, MediaSession, Notification, Os, Platform, PlatformError,
    TrayHandler, TrayImage, TrayMenuState, bgra_to_rgba, collect_files, dedup_apps, find_in_path,
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

pub(super) fn begin_window_drag(raw_window: u64, drag: super::WindowDrag) {
    use super::WindowDrag;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT, HTRIGHT, HTTOP,
        HTTOPLEFT, HTTOPRIGHT, PostMessageW, WM_NCLBUTTONDOWN,
    };
    let hit = match drag {
        WindowDrag::Move => HTCAPTION,
        WindowDrag::North => HTTOP,
        WindowDrag::South => HTBOTTOM,
        WindowDrag::East => HTRIGHT,
        WindowDrag::West => HTLEFT,
        WindowDrag::NorthEast => HTTOPRIGHT,
        WindowDrag::NorthWest => HTTOPLEFT,
        WindowDrag::SouthEast => HTBOTTOMRIGHT,
        WindowDrag::SouthWest => HTBOTTOMLEFT,
    };
    unsafe {
        let mut point = std::mem::zeroed();
        GetCursorPos(&mut point);
        let packed = ((point.y as u16 as u32) << 16) | (point.x as u16 as u32);
        ReleaseCapture();
        PostMessageW(
            raw_window as _,
            WM_NCLBUTTONDOWN,
            hit as usize,
            packed as isize,
        );
    }
}

pub(super) fn notification_app_id() -> &'static str {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
    static REGISTERED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    REGISTERED.get_or_init(|| {
        let id = match crate::channel::Channel::current() {
            crate::channel::Channel::Dev => "VitorAlcantara.HubPessoal.Dev",
            crate::channel::Channel::Release => "VitorAlcantara.HubPessoal",
        };
        let key = wide(OsStr::new(&format!(r"Software\Classes\AppUserModelId\{id}")));
        let name = wide(OsStr::new("DisplayName"));
        let value = wide(OsStr::new(crate::channel::app_name()));
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        if status != 0 {
            tracing::warn!(status, "falha ao registrar o app nas notificações do Windows");
        }
        id
    })
}

pub(super) fn transparency_effects_enabled() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let key = wide(OsStr::new(
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
    ));
    let name = wide(OsStr::new("EnableTransparency"));
    let mut value: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    status != 0 || value != 0
}

pub(super) fn apply_taskbar_icon(raw_window: u64) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, ICON_BIG, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CYICON,
        SendMessageW, WM_SETICON,
    };
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let icon = LoadImageW(
            module,
            1 as _,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXICON),
            GetSystemMetrics(SM_CYICON),
            LR_DEFAULTCOLOR,
        );
        if !icon.is_null() {
            SendMessageW(
                raw_window as _,
                WM_SETICON,
                ICON_BIG as usize,
                icon as isize,
            );
        }
    }
}

struct WindowHandle(isize);

impl raw_window_handle::HasWindowHandle for WindowHandle {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let hwnd = std::num::NonZeroIsize::new(self.0)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let raw = raw_window_handle::RawWindowHandle::Win32(
            raw_window_handle::Win32WindowHandle::new(hwnd),
        );
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

pub(super) fn apply_acrylic(raw_window: u64, tint: (u8, u8, u8, u8)) -> Result<(), PlatformError> {
    window_vibrancy::apply_acrylic(WindowHandle(raw_window as isize), Some(tint)).map_err(|error| {
        PlatformError::Launch {
            target: "efeito acrílico".to_string(),
            reason: error.to_string(),
        }
    })
}

pub(super) fn apply_blur(raw_window: u64, tint: (u8, u8, u8, u8)) -> Result<(), PlatformError> {
    window_vibrancy::apply_blur(WindowHandle(raw_window as isize), Some(tint)).map_err(|error| {
        PlatformError::Launch {
            target: "desfoque da janela".to_string(),
            reason: error.to_string(),
        }
    })
}

pub(super) fn clear_blur(raw_window: u64) {
    let _ = window_vibrancy::clear_blur(WindowHandle(raw_window as isize));
}

pub(super) fn clear_acrylic(raw_window: u64) {
    let _ = window_vibrancy::clear_acrylic(WindowHandle(raw_window as isize));
}

pub(super) fn desktop_wallpaper() -> Option<PathBuf> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETDESKWALLPAPER, SystemParametersInfoW,
    };
    let mut buffer = [0u16; 260];
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            buffer.as_mut_ptr() as *mut _,
            0,
        )
    };
    if ok != 0 {
        let length = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        if length > 0 {
            let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    let fallback = PathBuf::from(std::env::var_os("APPDATA")?)
        .join("Microsoft")
        .join("Windows")
        .join("Themes")
        .join("TranscodedWallpaper");
    fallback.is_file().then_some(fallback)
}

pub(super) fn launch_installer(installer: &Path, relaunch: &Path) -> Result<(), PlatformError> {
    fn quote(path: &Path) -> String {
        let text = path
            .to_string_lossy()
            .replace(['\'', '\u{2018}', '\u{2019}'], "''");
        format!("'{text}'")
    }
    let script = format!(
        "Start-Sleep -Seconds 2; Start-Process -FilePath {} -ArgumentList '/S' -Wait; Start-Process -FilePath {}",
        quote(installer),
        quote(relaunch)
    );
    let mut command = std::process::Command::new("powershell");
    command
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command"])
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    super::hide_console_std(&mut command);
    command.spawn().map_err(|error| PlatformError::Launch {
        target: installer.display().to_string(),
        reason: error.to_string(),
    })?;
    Ok(())
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

static TRAY_THREAD: AtomicU32 = AtomicU32::new(0);

fn run_tray_loop(apply: &mut dyn FnMut()) {
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
        TRAY_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
        apply();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_APP {
                apply();
                continue;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn wake_tray() {
    let thread = TRAY_THREAD.load(Ordering::SeqCst);
    if thread != 0 {
        unsafe {
            PostThreadMessageW(thread, WM_APP, 0, 0);
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

pub(super) fn program_dirs(name: &str) -> Vec<PathBuf> {
    let name = name.to_lowercase();
    let env_dir = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let mut dirs = Vec::new();
    let matching = |base: PathBuf| {
        let Ok(entries) = std::fs::read_dir(&base) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(|folder| folder.to_lowercase().contains(&name))
            })
            .collect::<Vec<_>>()
    };
    let local = env_dir("LOCALAPPDATA");
    let bases = [
        env_dir("ProgramFiles"),
        env_dir("ProgramFiles(x86)"),
        local.as_ref().map(|dir| dir.join("Programs")),
        local
            .as_ref()
            .map(|dir| dir.join("Microsoft").join("WinGet").join("Packages")),
    ];
    for base in bases.into_iter().flatten() {
        for folder in matching(base) {
            dirs.push(folder.clone());
            if let Ok(entries) = std::fs::read_dir(&folder) {
                dirs.extend(
                    entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| path.is_dir()),
                );
            }
        }
    }
    if let Some(local) = &local {
        dirs.push(local.join("Microsoft").join("WinGet").join("Links"));
    }
    if let Some(home) = env_dir("USERPROFILE") {
        dirs.push(home.join("scoop").join("shims"));
    }
    if let Some(data) = env_dir("ProgramData") {
        dirs.push(data.join("chocolatey").join("bin"));
    }
    dirs
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

    fn update_tray(&self, state: TrayMenuState) {
        super::tray::queue(state);
        wake_tray();
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
