use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{InstalledApp, MediaSession, Os, Platform, PlatformError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCall {
    OpenProgram(String),
    OpenPath(PathBuf),
    OpenUrl(String),
    ListInstalledApps,
}

pub struct FakePlatform {
    os: Os,
    apps: Vec<InstalledApp>,
    fullscreen: AtomicBool,
    calls: Mutex<Vec<PlatformCall>>,
}

impl FakePlatform {
    pub fn new(os: Os) -> Self {
        Self {
            os,
            apps: Vec::new(),
            fullscreen: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with_apps(mut self, apps: Vec<InstalledApp>) -> Self {
        self.apps = apps;
        self
    }

    pub fn set_fullscreen(&self, value: bool) {
        self.fullscreen.store(value, Ordering::SeqCst);
    }

    pub fn calls(&self) -> Vec<PlatformCall> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn record(&self, call: PlatformCall) {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(call);
    }
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new(Os::current())
    }
}

impl Platform for FakePlatform {
    fn os(&self) -> Os {
        self.os
    }

    fn open_program(&self, target: &str) -> Result<(), PlatformError> {
        self.record(PlatformCall::OpenProgram(target.to_string()));
        Ok(())
    }

    fn open_path(&self, path: &Path) -> Result<(), PlatformError> {
        self.record(PlatformCall::OpenPath(path.to_path_buf()));
        Ok(())
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.record(PlatformCall::OpenUrl(url.to_string()));
        Ok(())
    }

    fn list_installed_apps(&self) -> Result<Vec<InstalledApp>, PlatformError> {
        self.record(PlatformCall::ListInstalledApps);
        Ok(self.apps.clone())
    }

    fn media_session(&self) -> Result<Box<dyn MediaSession>, PlatformError> {
        Err(PlatformError::Unsupported("sessão de mídia"))
    }

    fn is_fullscreen_app_running(&self) -> bool {
        self.fullscreen.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_calls_in_order() {
        let platform = FakePlatform::default();
        platform.open_program("code").unwrap();
        platform.open_path(Path::new("pasta")).unwrap();
        platform.open_url("https://exemplo.com").unwrap();
        assert_eq!(
            platform.calls(),
            vec![
                PlatformCall::OpenProgram("code".into()),
                PlatformCall::OpenPath(PathBuf::from("pasta")),
                PlatformCall::OpenUrl("https://exemplo.com".into()),
            ]
        );
    }

    #[test]
    fn returns_configured_apps_and_fullscreen_state() {
        let app = InstalledApp {
            name: "VS Code".into(),
            target: "code".into(),
        };
        let platform = FakePlatform::new(Os::Linux).with_apps(vec![app.clone()]);
        assert_eq!(platform.list_installed_apps().unwrap(), vec![app]);
        assert!(!platform.is_fullscreen_app_running());
        platform.set_fullscreen(true);
        assert!(platform.is_fullscreen_app_running());
    }

    #[test]
    fn script_command_uses_configured_os() {
        let platform = FakePlatform::new(Os::Linux);
        let spec = platform.script_command(Path::new("a.py")).unwrap();
        assert_eq!(spec.program, "python3");
    }
}
