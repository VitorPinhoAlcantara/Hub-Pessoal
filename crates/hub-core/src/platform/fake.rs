use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{
    ActionHandler, InstalledApp, MediaSession, Notification, Os, Platform, PlatformError,
    TrayEvent, TrayHandler, TrayImage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCall {
    OpenProgram(String),
    OpenPath(PathBuf),
    OpenUrl(String),
    ListInstalledApps,
    Notify(Notification),
    SpawnTray,
}

pub struct FakePlatform {
    os: Os,
    apps: Vec<InstalledApp>,
    fullscreen: AtomicBool,
    calls: Mutex<Vec<PlatformCall>>,
    pending_actions: Mutex<Vec<Option<ActionHandler>>>,
    tray: Mutex<Option<TrayHandler>>,
}

impl FakePlatform {
    pub fn new(os: Os) -> Self {
        Self {
            os,
            apps: Vec::new(),
            fullscreen: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            pending_actions: Mutex::new(Vec::new()),
            tray: Mutex::new(None),
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

    pub fn notifications(&self) -> Vec<Notification> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                PlatformCall::Notify(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    pub fn answer_notification(&self, index: usize, action: Option<&str>) -> bool {
        let handler = self
            .pending_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(index)
            .and_then(Option::take);
        match handler {
            Some(handler) => {
                handler(action.map(str::to_string));
                true
            }
            None => false,
        }
    }

    pub fn click_tray(&self, event: TrayEvent) -> bool {
        match self.tray.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(handler) => {
                handler(event);
                true
            }
            None => false,
        }
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

    fn notify(
        &self,
        notification: Notification,
        on_action: Option<ActionHandler>,
    ) -> Result<(), PlatformError> {
        self.record(PlatformCall::Notify(notification));
        self.pending_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(on_action);
        Ok(())
    }

    fn spawn_tray(&self, _image: TrayImage, handler: TrayHandler) -> Result<(), PlatformError> {
        self.record(PlatformCall::SpawnTray);
        *self.tray.lock().unwrap_or_else(|e| e.into_inner()) = Some(handler);
        Ok(())
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

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

    #[test]
    fn notification_actions_can_be_answered_once() {
        let platform = FakePlatform::default();
        let received = Arc::new(Mutex::new(None));
        let sink = received.clone();
        platform
            .notify(
                Notification {
                    title: "t".into(),
                    ..Notification::default()
                },
                Some(Box::new(move |action| {
                    *sink.lock().unwrap() = action;
                })),
            )
            .unwrap();
        assert_eq!(platform.notifications().len(), 1);
        assert!(platform.answer_notification(0, Some("concluir")));
        assert!(!platform.answer_notification(0, Some("concluir")));
        assert_eq!(received.lock().unwrap().as_deref(), Some("concluir"));
    }

    #[test]
    fn tray_clicks_reach_handler() {
        let platform = FakePlatform::default();
        assert!(!platform.click_tray(TrayEvent::Open));
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        platform
            .spawn_tray(
                TrayImage {
                    rgba: vec![],
                    width: 0,
                    height: 0,
                },
                Box::new(move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        assert!(platform.click_tray(TrayEvent::Quit));
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
