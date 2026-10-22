mod view;

use hub_core::HubContext;
use hub_core::per_os::PerOs;
use hub_core::platform::{InstalledApp, Os};
use hub_core::shortcuts::{self, DEFAULT_GROUP, Loaded, Shortcut, ShortcutKind};
use hub_ui_kit::Section;
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::{Element, Event, Subscription, Task, event};

#[derive(Debug, Clone)]
pub enum FormMessage {
    NameChanged(String),
    KindSelected(ShortcutKind),
    WindowsChanged(String),
    LinuxChanged(String),
    GroupChanged(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutForm {
    pub editing: Option<usize>,
    pub name: String,
    pub kind: ShortcutKind,
    pub windows: String,
    pub linux: String,
    pub group: String,
}

impl ShortcutForm {
    pub fn new() -> Self {
        Self {
            editing: None,
            name: String::new(),
            kind: ShortcutKind::Program,
            windows: String::new(),
            linux: String::new(),
            group: DEFAULT_GROUP.to_string(),
        }
    }

    pub fn edit(index: usize, shortcut: &Shortcut) -> Self {
        Self {
            editing: Some(index),
            name: shortcut.name.clone(),
            kind: shortcut.kind,
            windows: shortcut.target.windows().unwrap_or_default().to_string(),
            linux: shortcut.target.linux().unwrap_or_default().to_string(),
            group: shortcut.group.clone(),
        }
    }

    pub fn from_app(app: &InstalledApp, os: Os) -> Self {
        let mut form = Self::new();
        form.name = app.name.clone();
        form.group = "Programas".to_string();
        match os {
            Os::Windows => form.windows = app.target.clone(),
            Os::Linux => form.linux = app.target.clone(),
        }
        form
    }

    pub fn update(&mut self, message: FormMessage) {
        match message {
            FormMessage::NameChanged(name) => self.name = name,
            FormMessage::KindSelected(kind) => self.kind = kind,
            FormMessage::WindowsChanged(value) => self.windows = value,
            FormMessage::LinuxChanged(value) => self.linux = value,
            FormMessage::GroupChanged(group) => self.group = group,
        }
    }

    pub fn to_shortcut(&self) -> Shortcut {
        let windows = self.windows.trim();
        let linux = self.linux.trim();
        let target = if !windows.is_empty() && windows == linux {
            PerOs::Same(windows.to_string())
        } else {
            PerOs::split(Some(windows.to_string()), Some(linux.to_string()))
        };
        Shortcut {
            name: self.name.clone(),
            kind: self.kind,
            target,
            group: self.group.clone(),
        }
    }
}

impl Default for ShortcutForm {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Suggest {
    pub apps: Option<Vec<InstalledApp>>,
    pub query: String,
    pub error: Option<String>,
}

impl Suggest {
    pub fn filtered(&self) -> Vec<(usize, &InstalledApp)> {
        let query = self.query.trim().to_lowercase();
        self.apps
            .iter()
            .flatten()
            .enumerate()
            .filter(|(_, app)| query.is_empty() || app.name.to_lowercase().contains(&query))
            .collect()
    }
}

pub enum Modal {
    Form(ShortcutForm),
    Suggest(Suggest),
    ConfirmDelete(usize),
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    QueryChanged(String),
    Open(usize),
    NewShortcut,
    Edit(usize),
    Form(FormMessage),
    FormSaved,
    AskDelete(usize),
    DeleteConfirmed,
    SuggestOpened,
    AppsLoaded(Result<Vec<InstalledApp>, String>),
    SuggestQueryChanged(String),
    AppPicked(usize),
    ModalClosed,
    ErrorDismissed,
}

pub struct Shortcuts {
    ctx: HubContext,
    loaded: Loaded,
    query: String,
    modal: Option<Modal>,
    form_error: Option<String>,
    error: Option<String>,
}

impl Shortcuts {
    pub fn new(ctx: HubContext) -> Self {
        let loaded = ctx.shortcuts();
        Self {
            ctx,
            loaded,
            query: String::new(),
            modal: None,
            form_error: None,
            error: None,
        }
    }

    pub fn shortcuts(&self) -> &[Shortcut] {
        &self.loaded.shortcuts
    }

    pub fn problem(&self) -> Option<&str> {
        self.loaded.problem.as_deref()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn os(&self) -> Os {
        self.ctx.platform.os()
    }

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn form_error(&self) -> Option<&str> {
        self.form_error.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn grouped(&self) -> Vec<(String, Vec<(usize, &Shortcut)>)> {
        let groups = shortcuts::groups(self.shortcuts());
        groups
            .into_iter()
            .map(|group| {
                let items: Vec<(usize, &Shortcut)> = self
                    .shortcuts()
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.group == group && shortcuts::matches(s, &self.query))
                    .collect();
                (group, items)
            })
            .filter(|(_, items)| !items.is_empty())
            .collect()
    }

    pub fn known_groups(&self) -> Vec<String> {
        shortcuts::groups(self.shortcuts())
    }

    fn reload(&mut self) {
        self.loaded = self.ctx.shortcuts();
    }

    fn save_form(&mut self) {
        let Some(Modal::Form(form)) = &self.modal else {
            return;
        };
        let shortcut = form.to_shortcut();
        let path = self.ctx.paths.shortcuts_file();
        let result = {
            let mut store = self.ctx.shortcut_store();
            match form.editing {
                Some(index) => store.update(&path, index, shortcut),
                None => store.add(&path, shortcut).map(|_| ()),
            }
        };
        match result {
            Ok(()) => {
                self.modal = None;
                self.form_error = None;
                self.reload();
            }
            Err(error) => self.form_error = Some(error.to_string()),
        }
    }
}

fn listen(
    event: Event,
    _status: event::Status,
    _window: hub_ui_kit::iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key::Named::Escape),
            ..
        }) => Some(Message::ModalClosed),
        _ => None,
    }
}

impl Section for Shortcuts {
    type Message = Message;

    fn title(&self) -> &str {
        "Atalhos"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => self.reload(),
            Message::QueryChanged(query) => self.query = query,
            Message::Open(index) => {
                if let Some(shortcut) = self.shortcuts().get(index)
                    && let Err(error) = shortcuts::open(self.ctx.platform.as_ref(), shortcut)
                {
                    self.error = Some(format!("Não foi possível abrir {}: {error}", shortcut.name));
                }
            }
            Message::NewShortcut => {
                self.form_error = None;
                self.modal = Some(Modal::Form(ShortcutForm::new()));
            }
            Message::Edit(index) => {
                if let Some(form) = self
                    .shortcuts()
                    .get(index)
                    .map(|s| ShortcutForm::edit(index, s))
                {
                    self.form_error = None;
                    self.modal = Some(Modal::Form(form));
                }
            }
            Message::Form(message) => {
                if let Some(Modal::Form(form)) = &mut self.modal {
                    form.update(message);
                    self.form_error = None;
                }
            }
            Message::FormSaved => self.save_form(),
            Message::AskDelete(index) => self.modal = Some(Modal::ConfirmDelete(index)),
            Message::DeleteConfirmed => {
                if let Some(Modal::ConfirmDelete(index)) = self.modal.take() {
                    let path = self.ctx.paths.shortcuts_file();
                    let result = self.ctx.shortcut_store().remove(&path, index);
                    if let Err(error) = result {
                        self.error = Some(error.to_string());
                    }
                    self.reload();
                }
            }
            Message::SuggestOpened => {
                self.modal = Some(Modal::Suggest(Suggest {
                    apps: None,
                    query: String::new(),
                    error: None,
                }));
                let platform = self.ctx.platform.clone();
                return Task::perform(
                    async move { platform.list_installed_apps().map_err(|e| e.to_string()) },
                    Message::AppsLoaded,
                );
            }
            Message::AppsLoaded(result) => {
                if let Some(Modal::Suggest(suggest)) = &mut self.modal {
                    match result {
                        Ok(apps) => suggest.apps = Some(apps),
                        Err(error) => {
                            suggest.apps = Some(Vec::new());
                            suggest.error = Some(error);
                        }
                    }
                }
            }
            Message::SuggestQueryChanged(query) => {
                if let Some(Modal::Suggest(suggest)) = &mut self.modal {
                    suggest.query = query;
                }
            }
            Message::AppPicked(index) => {
                let os = self.os();
                let form = match &self.modal {
                    Some(Modal::Suggest(suggest)) => suggest
                        .apps
                        .as_ref()
                        .and_then(|apps| apps.get(index))
                        .map(|app| ShortcutForm::from_app(app, os)),
                    _ => None,
                };
                if let Some(form) = form {
                    self.form_error = None;
                    self.modal = Some(Modal::Form(form));
                }
            }
            Message::ModalClosed => self.modal = None,
            Message::ErrorDismissed => self.error = None,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        event::listen_with(listen)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hub_core::clock::FakeClock;
    use hub_core::platform::{FakePlatform, PlatformCall};
    use hub_core::{Config, Database, HubPaths};

    use super::*;

    fn setup(apps: Vec<InstalledApp>) -> (Shortcuts, Arc<FakePlatform>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path());
        paths.ensure_dirs().unwrap();
        let platform = Arc::new(FakePlatform::new(Os::Windows).with_apps(apps));
        let ctx = HubContext::new(
            paths,
            Config::default(),
            Database::open_in_memory().unwrap(),
            platform.clone(),
            Arc::new(FakeClock::at("2026-10-21 10:00")),
        );
        (Shortcuts::new(ctx), platform, temp)
    }

    fn send(section: &mut Shortcuts, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            let _ = section.update(message);
        }
    }

    fn add(section: &mut Shortcuts, name: &str, kind: ShortcutKind, target: &str, group: &str) {
        send(
            section,
            [
                Message::NewShortcut,
                Message::Form(FormMessage::NameChanged(name.into())),
                Message::Form(FormMessage::KindSelected(kind)),
                Message::Form(FormMessage::WindowsChanged(target.into())),
                Message::Form(FormMessage::LinuxChanged(target.into())),
                Message::Form(FormMessage::GroupChanged(group.into())),
                Message::FormSaved,
            ],
        );
    }

    #[test]
    fn adding_groups_and_searching() {
        let (mut section, _, _temp) = setup(Vec::new());
        add(
            &mut section,
            "VS Code",
            ShortcutKind::Program,
            "code",
            "Dev",
        );
        add(
            &mut section,
            "Projetos",
            ShortcutKind::Folder,
            "~/projetos",
            "Pastas",
        );
        add(
            &mut section,
            "GitHub",
            ShortcutKind::Url,
            "github.com",
            "Dev",
        );
        assert!(section.modal().is_none());
        let groups = section.grouped();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "Dev");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(section.shortcuts()[0].target, PerOs::Same("code".into()));
        send(&mut section, [Message::QueryChanged("git".into())]);
        let groups = section.grouped();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1[0].1.name, "GitHub");
    }

    #[test]
    fn invalid_form_shows_error() {
        let (mut section, _, _temp) = setup(Vec::new());
        send(&mut section, [Message::NewShortcut, Message::FormSaved]);
        assert!(section.form_error().is_some());
        assert!(matches!(section.modal(), Some(Modal::Form(_))));
    }

    #[test]
    fn open_edit_and_delete() {
        let (mut section, platform, _temp) = setup(Vec::new());
        add(
            &mut section,
            "Site",
            ShortcutKind::Url,
            "exemplo.com",
            "Web",
        );
        send(&mut section, [Message::Open(0)]);
        assert_eq!(
            platform.calls(),
            vec![PlatformCall::OpenUrl("https://exemplo.com".into())]
        );
        send(
            &mut section,
            [
                Message::Edit(0),
                Message::Form(FormMessage::NameChanged("Site novo".into())),
                Message::FormSaved,
            ],
        );
        assert_eq!(section.shortcuts()[0].name, "Site novo");
        send(&mut section, [Message::AskDelete(0)]);
        assert_eq!(section.shortcuts().len(), 1);
        send(&mut section, [Message::DeleteConfirmed]);
        assert!(section.shortcuts().is_empty());
    }

    #[test]
    fn suggestions_prefill_form() {
        let apps = vec![
            InstalledApp {
                name: "Firefox".into(),
                target: "C:\\firefox.lnk".into(),
            },
            InstalledApp {
                name: "Steam".into(),
                target: "C:\\steam.lnk".into(),
            },
        ];
        let (mut section, _, _temp) = setup(apps.clone());
        send(
            &mut section,
            [
                Message::SuggestOpened,
                Message::AppsLoaded(Ok(apps)),
                Message::SuggestQueryChanged("ste".into()),
            ],
        );
        let Some(Modal::Suggest(suggest)) = section.modal() else {
            panic!("esperava sugestões")
        };
        let filtered = suggest.filtered();
        assert_eq!(filtered.len(), 1);
        let index = filtered[0].0;
        send(
            &mut section,
            [Message::AppPicked(index), Message::FormSaved],
        );
        let saved = &section.shortcuts()[0];
        assert_eq!(saved.name, "Steam");
        assert_eq!(saved.target.windows(), Some("C:\\steam.lnk"));
        assert_eq!(saved.target.linux(), None);
    }

    #[test]
    fn view_builds_in_every_state() {
        let (mut section, _, _temp) = setup(Vec::new());
        let _ = section.view();
        add(&mut section, "A", ShortcutKind::Command, "echo oi", "G");
        let _ = section.view();
        send(&mut section, [Message::SuggestOpened]);
        let _ = section.view();
        send(&mut section, [Message::AppsLoaded(Ok(Vec::new()))]);
        let _ = section.view();
        send(&mut section, [Message::Edit(0)]);
        let _ = section.view();
        send(&mut section, [Message::AskDelete(0)]);
        let _ = section.view();
    }
}
