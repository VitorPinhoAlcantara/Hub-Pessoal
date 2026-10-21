use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowRect, SW_SHOWNORMAL,
};

use super::{InstalledApp, MediaSession, Os, Platform, PlatformError, collect_files, dedup_apps};

pub struct WindowsPlatform;

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

    fn media_session(&self) -> Result<Box<dyn MediaSession>, PlatformError> {
        Err(PlatformError::Unsupported("sessão de mídia"))
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
    fn listing_installed_apps_does_not_fail() {
        assert!(WindowsPlatform.list_installed_apps().is_ok());
    }
}
