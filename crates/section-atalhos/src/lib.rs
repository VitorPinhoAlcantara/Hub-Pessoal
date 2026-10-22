mod view;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hub_core::HubContext;
use hub_core::per_os::PerOs;
use hub_core::platform::{self, FileIcon, InstalledApp, Os, Platform};
use hub_core::shortcuts::{self, DEFAULT_GROUP, Loaded, Shortcut, ShortcutKind};
use hub_ui_kit::iced::futures::{Stream, stream};
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::{Element, Event, Subscription, Task, event};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section, dialog};

pub const VISIBLE_APPS: usize = 60;
const ICON_BATCH: usize = 16;

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

    pub fn update(&mut self, message: FormMessage) {
        match message {
            FormMessage::NameChanged(name) => self.name = name,
            FormMessage::KindSelected(kind) => self.kind = kind,
            FormMessage::WindowsChanged(value) => self.windows = value,
            FormMessage::LinuxChanged(value) => self.linux = value,
            FormMessage::GroupChanged(group) => self.group = group,
        }
    }

    fn target_mut(&mut self, os: Os) -> &mut String {
        match os {
            Os::Windows => &mut self.windows,
            Os::Linux => &mut self.linux,
        }
    }

    pub fn apply_app(&mut self, app: &InstalledApp, os: Os) {
        self.kind = ShortcutKind::Program;
        self.name = app.name.clone();
        *self.target_mut(os) = app.target.clone();
    }

    pub fn apply_path(&mut self, path: &Path, kind: ShortcutKind, os: Os) {
        self.kind = kind;
        *self.target_mut(os) = path.to_string_lossy().into_owned();
        if self.name.trim().is_empty() {
            let name = match kind {
                ShortcutKind::Folder => path.file_name(),
                _ => path.file_stem(),
            };
            if let Some(name) = name {
                self.name = name.to_string_lossy().into_owned();
            }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Apps {
    Idle,
    Loading,
    Ready(Vec<InstalledApp>),
    Failed(String),
}

pub enum Modal {
    Form(ShortcutForm),
    ConfirmDelete(usize),
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    QueryChanged(String),
    Open(usize),
    Reveal(usize),
    NewShortcut,
    Edit(usize),
    CardMenu(usize),
    AreaMenu,
    MenuClosed,
    Form(FormMessage),
    FormSaved,
    AskDelete(usize),
    DeleteConfirmed,
    AppsLoaded(Result<Vec<InstalledApp>, String>),
    AppQueryChanged(String),
    AppPicked(usize),
    PickFile,
    PickFolder,
    FilePicked(Option<PathBuf>),
    FolderPicked(Option<PathBuf>),
    IconsLoaded(Vec<(PathBuf, Option<Handle>)>),
    ModalClosed,
    ErrorDismissed,
}

pub struct Shortcuts {
    ctx: HubContext,
    loaded: Loaded,
    query: String,
    modal: Option<Modal>,
    menu: Option<ContextMenu<Message>>,
    icons: HashMap<PathBuf, Option<Handle>>,
    apps: Apps,
    app_query: String,
    form_error: Option<String>,
    error: Option<String>,
}

pub fn ellipsize(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 3 {
        return text.to_string();
    }
    let keep = max - 1;
    let head = keep / 3;
    let tail = keep - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

pub fn icon_handle(icon: FileIcon) -> Handle {
    match icon {
        FileIcon::Rgba {
            width,
            height,
            pixels,
        } => Handle::from_rgba(width, height, pixels),
        FileIcon::Encoded(bytes) => Handle::from_bytes(bytes),
    }
}

impl Shortcuts {
    pub fn new(ctx: HubContext) -> Self {
        let loaded = ctx.shortcuts();
        Self {
            ctx,
            loaded,
            query: String::new(),
            modal: None,
            menu: None,
            icons: HashMap::new(),
            apps: Apps::Idle,
            app_query: String::new(),
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

    pub fn form(&self) -> Option<&ShortcutForm> {
        match &self.modal {
            Some(Modal::Form(form)) => Some(form),
            _ => None,
        }
    }

    pub fn form_error(&self) -> Option<&str> {
        self.form_error.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn apps(&self) -> &Apps {
        &self.apps
    }

    pub fn app_query(&self) -> &str {
        &self.app_query
    }

    pub fn visible_apps(&self) -> Vec<(usize, &InstalledApp)> {
        match &self.apps {
            Apps::Ready(apps) => {
                let mut found = platform::filter_apps(apps, &self.app_query);
                found.truncate(VISIBLE_APPS);
                found
            }
            _ => Vec::new(),
        }
    }

    pub fn icon(&self, path: &Path) -> Option<&Handle> {
        self.icons.get(path).and_then(Option::as_ref)
    }

    pub fn shortcut_icon(&self, shortcut: &Shortcut) -> Option<&Handle> {
        shortcuts::local_path(shortcut, self.os()).and_then(|path| self.icon(&path))
    }

    pub fn missing_icons(&self) -> Vec<PathBuf> {
        let os = self.os();
        let mut wanted: Vec<PathBuf> = self
            .shortcuts()
            .iter()
            .filter_map(|shortcut| shortcuts::local_path(shortcut, os))
            .collect();
        if self.form().is_some_and(|f| f.kind == ShortcutKind::Program) {
            wanted.extend(
                self.visible_apps()
                    .into_iter()
                    .map(|(_, app)| PathBuf::from(&app.target)),
            );
        }
        let mut missing = Vec::new();
        for path in wanted {
            if !self.icons.contains_key(&path) && !missing.contains(&path) {
                missing.push(path);
                if missing.len() == ICON_BATCH {
                    break;
                }
            }
        }
        missing
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

    fn card_menu(&self, index: usize) -> Option<ContextMenu<Message>> {
        let shortcut = self.shortcuts().get(index)?;
        let os = self.os();
        let open = MenuItem::new("Abrir", Message::Open(index)).icon(Icon::Play);
        let mut items = vec![if shortcut.target.for_os(os).is_some() {
            open
        } else {
            open.disabled()
        }];
        items.push(MenuItem::new("Editar", Message::Edit(index)).icon(Icon::Edit));
        if shortcuts::local_path(shortcut, os).is_some() {
            items.push(
                MenuItem::new("Abrir local do arquivo", Message::Reveal(index)).icon(Icon::Folder),
            );
        }
        items.push(
            MenuItem::new("Excluir", Message::AskDelete(index))
                .icon(Icon::Trash)
                .danger(),
        );
        Some(ContextMenu::at_cursor(items, Message::MenuClosed))
    }

    fn area_menu(&self) -> ContextMenu<Message> {
        ContextMenu::at_cursor(
            vec![MenuItem::new("Novo atalho", Message::NewShortcut).icon(Icon::Plus)],
            Message::MenuClosed,
        )
    }

    fn open_form(&mut self, form: ShortcutForm) -> Task<Message> {
        self.form_error = None;
        self.app_query.clear();
        self.modal = Some(Modal::Form(form));
        if matches!(self.apps, Apps::Idle | Apps::Failed(_)) {
            self.apps = Apps::Loading;
            let platform = self.ctx.platform.clone();
            return Task::perform(load_apps(platform), Message::AppsLoaded);
        }
        Task::none()
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

    fn apply_picked(&mut self, path: Option<PathBuf>, kind: ShortcutKind) {
        let os = self.os();
        if let (Some(path), Some(Modal::Form(form))) = (path, &mut self.modal) {
            form.apply_path(&path, kind, os);
            self.form_error = None;
        }
    }
}

async fn load_apps(platform: Arc<dyn Platform>) -> Result<Vec<InstalledApp>, String> {
    tokio::task::spawn_blocking(move || platform.list_installed_apps().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

struct IconJob {
    paths: Vec<PathBuf>,
    platform: Arc<dyn Platform>,
}

impl Hash for IconJob {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.paths.hash(state);
    }
}

fn load_icons(job: &IconJob) -> impl Stream<Item = Message> + use<> {
    let paths = job.paths.clone();
    let platform = job.platform.clone();
    stream::once(async move {
        let loaded = tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .map(|path| {
                    let icon = platform.file_icon(&path).map(icon_handle);
                    (path, icon)
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        Message::IconsLoaded(loaded)
    })
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
        if !matches!(
            message,
            Message::AppsLoaded(_) | Message::IconsLoaded(_) | Message::Refresh
        ) {
            self.menu = None;
        }
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
            Message::Reveal(index) => {
                if let Some(shortcut) = self.shortcuts().get(index)
                    && let Err(error) = shortcuts::reveal(self.ctx.platform.as_ref(), shortcut)
                {
                    self.error = Some(format!(
                        "Não foi possível mostrar {} na pasta: {error}",
                        shortcut.name
                    ));
                }
            }
            Message::NewShortcut => return self.open_form(ShortcutForm::new()),
            Message::Edit(index) => {
                if let Some(form) = self
                    .shortcuts()
                    .get(index)
                    .map(|s| ShortcutForm::edit(index, s))
                {
                    return self.open_form(form);
                }
            }
            Message::CardMenu(index) => self.menu = self.card_menu(index),
            Message::AreaMenu => {
                if self.modal.is_none() {
                    self.menu = Some(self.area_menu());
                }
            }
            Message::MenuClosed => {}
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
            Message::AppsLoaded(result) => {
                self.apps = match result {
                    Ok(apps) => Apps::Ready(apps),
                    Err(error) => Apps::Failed(error),
                };
            }
            Message::AppQueryChanged(query) => self.app_query = query,
            Message::AppPicked(index) => {
                let os = self.os();
                let app = match &self.apps {
                    Apps::Ready(apps) => apps.get(index).cloned(),
                    _ => None,
                };
                if let (Some(app), Some(Modal::Form(form))) = (app, &mut self.modal) {
                    form.apply_app(&app, os);
                    self.form_error = None;
                }
            }
            Message::PickFile => {
                return Task::perform(dialog::pick_file("Escolher arquivo"), Message::FilePicked);
            }
            Message::PickFolder => {
                return Task::perform(dialog::pick_folder("Escolher pasta"), Message::FolderPicked);
            }
            Message::FilePicked(path) => self.apply_picked(path, ShortcutKind::Program),
            Message::FolderPicked(path) => self.apply_picked(path, ShortcutKind::Folder),
            Message::IconsLoaded(icons) => self.icons.extend(icons),
            Message::ModalClosed => self.modal = None,
            Message::ErrorDismissed => self.error = None,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = event::listen_with(listen);
        let missing = self.missing_icons();
        if missing.is_empty() {
            return keys;
        }
        let job = IconJob {
            paths: missing,
            platform: self.ctx.platform.clone(),
        };
        Subscription::batch([keys, Subscription::run_with(job, load_icons)])
    }

    fn context_menu(&self) -> Option<ContextMenu<Message>> {
        self.menu.clone()
    }
}
