mod view;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hub_core::HubContext;
use hub_core::events::HubEvent;
use hub_core::notes::{self, Note, TempReset};
use hub_core::vault::{self, VaultError, VaultItem, VaultKey, VaultStore};
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::widget::text_editor;
use hub_ui_kit::iced::{Element, Event, Subscription, Task, event, time};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section, dialog};

pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);
pub const AUTOSAVE_POLL: Duration = Duration::from_millis(200);
pub const VAULT_IDLE: Duration = Duration::from_secs(300);
pub const VAULT_POLL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Notes,
    Vault,
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub wrong_password: bool,
    pub text: String,
}

impl Failure {
    fn other(text: impl Into<String>) -> Self {
        Self {
            wrong_password: false,
            text: text.into(),
        }
    }
}

impl From<VaultError> for Failure {
    fn from(error: VaultError) -> Self {
        Self {
            wrong_password: matches!(error, VaultError::WrongPassword),
            text: error.to_string(),
        }
    }
}

pub struct Editing {
    pub id: i64,
    pub title: String,
    pub content: text_editor::Content,
}

#[derive(Default)]
pub struct PasswordForm {
    pub old: String,
    pub new: String,
    pub confirm: String,
    pub error: Option<String>,
    pub busy: bool,
}

impl PasswordForm {
    pub fn can_submit(&self) -> bool {
        !self.busy && !self.old.is_empty() && !self.new.is_empty() && self.new == self.confirm
    }
}

pub enum Modal {
    ConfirmDeleteNote(i64),
    ChangePassword(PasswordForm),
    Rename { id: String, name: String },
    ConfirmRemoveItem(String),
}

#[derive(Debug, Clone)]
pub enum PasswordField {
    Old,
    New,
    Confirm,
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    QueryChanged(String),
    NewNote,
    NoteSelected(i64),
    TitleChanged(String),
    BodyAction(text_editor::Action),
    ToggleFavorite,
    FavoriteOf(i64),
    ResetSelected(TempReset),
    AskDeleteNote(i64),
    DeleteConfirmed,
    NoteMenu(i64),
    Autosave,
    PasswordChanged(String),
    ConfirmChanged(String),
    CreateVault,
    Unlock,
    VaultOpened(Result<(VaultKey, Vec<VaultItem>), Failure>),
    AddFiles,
    FilesPicked(Vec<PathBuf>),
    FilesAdded(Result<Vec<VaultItem>, Failure>, Option<String>),
    LockVault,
    VaultTick,
    OpenItem(String),
    TempReady(Result<PathBuf, Failure>),
    ItemMenu(String),
    ExportItem(String),
    ExportFolder(String, Option<PathBuf>),
    Exported(Result<(), Failure>),
    AskRename(String),
    RenameChanged(String),
    RenameConfirmed,
    AskRemove(String),
    RemoveConfirmed,
    OpenChangePassword,
    PasswordFieldChanged(PasswordField, String),
    PasswordSubmitted,
    PasswordUpdated(Result<(), Failure>),
    ModalClosed,
    ErrorDismissed,
    MenuClosed,
}

pub struct Notes {
    ctx: HubContext,
    tab: Tab,
    notes: Vec<Note>,
    query: String,
    editing: Option<Editing>,
    dirty_since: Option<Instant>,
    modal: Option<Modal>,
    error: Option<String>,
    menu: Option<ContextMenu<Message>>,
    store: VaultStore,
    vault_exists: bool,
    key: Option<VaultKey>,
    items: Vec<VaultItem>,
    password: String,
    confirm: String,
    wrong_password: bool,
    busy: Option<String>,
    vault_activity: Instant,
}

impl Notes {
    pub fn new(ctx: HubContext) -> Self {
        let store = VaultStore::new(&ctx.paths);
        let vault_exists = store.exists();
        let mut section = Self {
            ctx,
            tab: Tab::Notes,
            notes: Vec::new(),
            query: String::new(),
            editing: None,
            dirty_since: None,
            modal: None,
            error: None,
            menu: None,
            store,
            vault_exists,
            key: None,
            items: Vec::new(),
            password: String::new(),
            confirm: String::new(),
            wrong_password: false,
            busy: None,
            vault_activity: Instant::now(),
        };
        section.reload_list();
        section
    }

    pub fn refresh(&mut self) {
        self.reload_list();
        let Some(id) = self.editing.as_ref().map(|e| e.id) else {
            return;
        };
        if self.dirty_since.is_some() {
            return;
        }
        match self.query_note(id) {
            Some(note) => self.editing = Some(Self::editing_from(&note)),
            None => self.editing = None,
        }
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn editing(&self) -> Option<&Editing> {
        self.editing.as_ref()
    }

    pub fn selected(&self) -> Option<&Note> {
        let id = self.editing.as_ref()?.id;
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn vault_exists(&self) -> bool {
        self.vault_exists
    }

    pub fn vault_unlocked(&self) -> bool {
        self.key.is_some()
    }

    pub fn vault_items(&self) -> &[VaultItem] {
        &self.items
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn confirm(&self) -> &str {
        &self.confirm
    }

    pub fn wrong_password(&self) -> bool {
        self.wrong_password
    }

    pub fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    pub fn can_create(&self) -> bool {
        !self.password.is_empty() && self.password == self.confirm
    }

    pub fn now(&self) -> hub_core::chrono::NaiveDateTime {
        self.ctx.clock.now()
    }

    fn editing_from(note: &Note) -> Editing {
        Editing {
            id: note.id,
            title: note.title.clone(),
            content: text_editor::Content::with_text(&note.body),
        }
    }

    fn query_note(&self, id: i64) -> Option<Note> {
        notes::get(self.ctx.db().conn(), id).ok().flatten()
    }

    fn reload_list(&mut self) {
        let result = if self.query.trim().is_empty() {
            notes::list(self.ctx.db().conn())
        } else {
            notes::search(self.ctx.db().conn(), self.query.trim())
        };
        match result {
            Ok(mut list) => {
                list.sort_by_key(|n| !n.favorite);
                self.notes = list;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn publish(&self) {
        self.ctx.events.publish(HubEvent::NotesChanged);
    }

    fn save_pending(&mut self) {
        if self.dirty_since.take().is_none() {
            return;
        }
        let Some(editing) = &self.editing else {
            return;
        };
        let body = editing.content.text();
        let result = notes::update(
            self.ctx.db().conn(),
            editing.id,
            &editing.title,
            &body,
            self.ctx.clock.now(),
        );
        match result {
            Ok(()) => {
                self.reload_list();
                self.publish();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn select(&mut self, id: i64) {
        self.save_pending();
        match self.query_note(id) {
            Some(note) => self.editing = Some(Self::editing_from(&note)),
            None => self.editing = None,
        }
    }

    fn create_note(&mut self) {
        self.save_pending();
        self.query.clear();
        let result = notes::create(self.ctx.db().conn(), "", "", self.ctx.clock.now());
        match result {
            Ok(note) => {
                self.reload_list();
                self.editing = Some(Self::editing_from(&note));
                self.publish();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn set_favorite(&mut self, id: i64, favorite: bool) {
        self.save_pending();
        let result = notes::set_favorite(self.ctx.db().conn(), id, favorite);
        match result {
            Ok(()) => {
                self.reload_list();
                self.publish();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn set_reset(&mut self, reset: TempReset) {
        let Some(id) = self.editing.as_ref().map(|e| e.id) else {
            return;
        };
        self.save_pending();
        let result = notes::set_reset(self.ctx.db().conn(), id, reset, self.ctx.clock.now());
        match result {
            Ok(()) => {
                self.reload_list();
                self.publish();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn delete_note(&mut self, id: i64) {
        if self.editing.as_ref().is_some_and(|e| e.id == id) {
            self.editing = None;
            self.dirty_since = None;
        }
        let result = notes::delete(self.ctx.db().conn(), id);
        match result {
            Ok(()) => {
                self.reload_list();
                self.publish();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn note_menu(&self, id: i64) -> Option<ContextMenu<Message>> {
        let note = self.notes.iter().find(|n| n.id == id)?;
        Some(ContextMenu::at_cursor(
            vec![
                MenuItem::new("Abrir", Message::NoteSelected(id)).icon(Icon::Notes),
                MenuItem::new(
                    if note.favorite {
                        "Desfavoritar"
                    } else {
                        "Favoritar"
                    },
                    Message::FavoriteOf(id),
                )
                .icon(Icon::Star),
                MenuItem::new("Excluir", Message::AskDeleteNote(id))
                    .icon(Icon::Trash)
                    .danger(),
            ],
            Message::MenuClosed,
        ))
    }

    fn item(&self, id: &str) -> Option<&VaultItem> {
        self.items.iter().find(|i| i.id == id)
    }

    fn item_menu(&self, id: &str) -> Option<ContextMenu<Message>> {
        self.item(id)?;
        let id = id.to_string();
        Some(ContextMenu::at_cursor(
            vec![
                MenuItem::new("Abrir", Message::OpenItem(id.clone())).icon(Icon::External),
                MenuItem::new("Exportar", Message::ExportItem(id.clone())).icon(Icon::Folder),
                MenuItem::new("Renomear", Message::AskRename(id.clone())).icon(Icon::Edit),
                MenuItem::new("Remover do cofre", Message::AskRemove(id))
                    .icon(Icon::Trash)
                    .danger(),
            ],
            Message::MenuClosed,
        ))
    }

    fn lock(&mut self) {
        let was_unlocked = self.key.take().is_some();
        self.items.clear();
        self.password.clear();
        self.confirm.clear();
        self.busy = None;
        if matches!(
            self.modal,
            Some(Modal::ChangePassword(_) | Modal::Rename { .. } | Modal::ConfirmRemoveItem(_))
        ) {
            self.modal = None;
        }
        if was_unlocked {
            vault::clear_temp();
        }
    }

    fn reload_items(&mut self) {
        let Some(key) = self.key.clone() else {
            return;
        };
        match self.store.items(&key) {
            Ok(items) => self.items = items,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn vault_opened(&mut self, result: Result<(VaultKey, Vec<VaultItem>), Failure>) {
        self.busy = None;
        match result {
            Ok((key, items)) => {
                self.key = Some(key);
                self.items = items;
                self.vault_exists = true;
                self.password.clear();
                self.confirm.clear();
                self.wrong_password = false;
                self.vault_activity = Instant::now();
            }
            Err(failure) if failure.wrong_password => {
                self.wrong_password = true;
                self.password.clear();
            }
            Err(failure) => self.error = Some(failure.text),
        }
    }

    fn is_vault_message(message: &Message) -> bool {
        !matches!(
            message,
            Message::TabSelected(_)
                | Message::QueryChanged(_)
                | Message::NewNote
                | Message::NoteSelected(_)
                | Message::TitleChanged(_)
                | Message::BodyAction(_)
                | Message::ToggleFavorite
                | Message::FavoriteOf(_)
                | Message::ResetSelected(_)
                | Message::AskDeleteNote(_)
                | Message::DeleteConfirmed
                | Message::NoteMenu(_)
                | Message::Autosave
                | Message::VaultTick
                | Message::MenuClosed
                | Message::ModalClosed
                | Message::ErrorDismissed
        )
    }
}

impl Drop for Notes {
    fn drop(&mut self) {
        self.lock();
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, VaultError> + Send + 'static,
) -> Result<T, Failure> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(Failure::from),
        Err(error) => Err(Failure::other(error.to_string())),
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

impl Section for Notes {
    type Message = Message;

    fn title(&self) -> &str {
        "Anotações"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if !matches!(message, Message::NoteMenu(_) | Message::ItemMenu(_)) {
            self.menu = None;
        }
        if self.tab == Tab::Vault && self.key.is_some() && Self::is_vault_message(&message) {
            self.vault_activity = Instant::now();
        }
        match message {
            Message::TabSelected(tab) => {
                self.save_pending();
                self.tab = tab;
                self.modal = None;
                if tab == Tab::Vault {
                    self.vault_exists = self.store.exists();
                    self.vault_activity = Instant::now();
                }
            }
            Message::QueryChanged(query) => {
                self.query = query;
                self.reload_list();
            }
            Message::NewNote => self.create_note(),
            Message::NoteSelected(id) => self.select(id),
            Message::TitleChanged(title) => {
                if let Some(editing) = &mut self.editing {
                    editing.title = title;
                    self.dirty_since = Some(Instant::now());
                }
            }
            Message::BodyAction(action) => {
                if let Some(editing) = &mut self.editing {
                    let edit = action.is_edit();
                    editing.content.perform(action);
                    if edit {
                        self.dirty_since = Some(Instant::now());
                    }
                }
            }
            Message::ToggleFavorite => {
                if let Some(note) = self.selected() {
                    let (id, favorite) = (note.id, !note.favorite);
                    self.set_favorite(id, favorite);
                }
            }
            Message::FavoriteOf(id) => {
                if let Some(note) = self.notes.iter().find(|n| n.id == id) {
                    let favorite = !note.favorite;
                    self.set_favorite(id, favorite);
                }
            }
            Message::ResetSelected(reset) => self.set_reset(reset),
            Message::AskDeleteNote(id) => self.modal = Some(Modal::ConfirmDeleteNote(id)),
            Message::DeleteConfirmed => {
                if let Some(Modal::ConfirmDeleteNote(id)) = self.modal.take() {
                    self.delete_note(id);
                }
            }
            Message::NoteMenu(id) => self.menu = self.note_menu(id),
            Message::Autosave => {
                if self
                    .dirty_since
                    .is_some_and(|since| since.elapsed() >= AUTOSAVE_DELAY)
                {
                    self.save_pending();
                }
            }
            Message::PasswordChanged(value) => {
                self.password = value;
                self.wrong_password = false;
            }
            Message::ConfirmChanged(value) => self.confirm = value,
            Message::CreateVault => {
                if self.vault_exists || !self.can_create() || self.busy.is_some() {
                    return Task::none();
                }
                let store = self.store.clone();
                let password = std::mem::take(&mut self.password);
                self.confirm.clear();
                self.busy = Some("Criando cofre".to_string());
                return Task::perform(
                    blocking(move || {
                        let key = store.create(&password)?;
                        let items = store.items(&key)?;
                        Ok((key, items))
                    }),
                    Message::VaultOpened,
                );
            }
            Message::Unlock => {
                if !self.vault_exists
                    || self.key.is_some()
                    || self.password.is_empty()
                    || self.busy.is_some()
                {
                    return Task::none();
                }
                let store = self.store.clone();
                let password = std::mem::take(&mut self.password);
                self.wrong_password = false;
                self.busy = Some("Desbloqueando".to_string());
                return Task::perform(
                    blocking(move || {
                        let key = store.unlock(&password)?;
                        let items = store.items(&key)?;
                        Ok((key, items))
                    }),
                    Message::VaultOpened,
                );
            }
            Message::VaultOpened(result) => self.vault_opened(result),
            Message::AddFiles => {
                if self.key.is_some() && self.busy.is_none() {
                    return Task::perform(dialog::pick_files(), Message::FilesPicked);
                }
            }
            Message::FilesPicked(paths) => {
                let Some(key) = self.key.clone() else {
                    return Task::none();
                };
                if paths.is_empty() {
                    return Task::none();
                }
                self.busy = Some(match paths.len() {
                    1 => "Adicionando 1 arquivo".to_string(),
                    n => format!("Adicionando {n} arquivos"),
                });
                let store = self.store.clone();
                let now = self.ctx.clock.now();
                return Task::perform(
                    async move {
                        let outcome = blocking(move || {
                            let mut first_error = None;
                            for path in &paths {
                                if let Err(error) = store.add_file(&key, path, now)
                                    && first_error.is_none()
                                {
                                    first_error = Some(error.to_string());
                                }
                            }
                            Ok((store.items(&key)?, first_error))
                        })
                        .await;
                        match outcome {
                            Ok((items, error)) => (Ok(items), error),
                            Err(failure) => (Err(failure), None),
                        }
                    },
                    |(items, error)| Message::FilesAdded(items, error),
                );
            }
            Message::FilesAdded(result, error) => {
                self.busy = None;
                if self.key.is_some() {
                    match result {
                        Ok(items) => self.items = items,
                        Err(failure) => self.error = Some(failure.text),
                    }
                }
                if let Some(error) = error {
                    self.error = Some(error);
                }
            }
            Message::LockVault => self.lock(),
            Message::VaultTick => {
                if self.key.is_some() && self.vault_activity.elapsed() >= VAULT_IDLE {
                    self.lock();
                }
            }
            Message::OpenItem(id) => {
                let Some(key) = self.key.clone() else {
                    return Task::none();
                };
                let store = self.store.clone();
                return Task::perform(
                    blocking(move || store.open_temp(&key, &id)),
                    Message::TempReady,
                );
            }
            Message::TempReady(result) => match result {
                Ok(path) => {
                    if let Err(error) = self.ctx.platform.open_path(&path) {
                        self.error = Some(error.to_string());
                    }
                }
                Err(failure) => self.error = Some(failure.text),
            },
            Message::ItemMenu(id) => self.menu = self.item_menu(&id),
            Message::ExportItem(id) => {
                return Task::perform(
                    dialog::pick_folder("Exportar para a pasta"),
                    move |folder| Message::ExportFolder(id.clone(), folder),
                );
            }
            Message::ExportFolder(id, folder) => {
                let (Some(folder), Some(key), Some(item)) =
                    (folder, self.key.clone(), self.item(&id).cloned())
                else {
                    return Task::none();
                };
                let store = self.store.clone();
                let dest = folder.join(&item.name);
                return Task::perform(
                    blocking(move || store.export(&key, &item.id, &dest)),
                    Message::Exported,
                );
            }
            Message::Exported(result) => {
                if let Err(failure) = result {
                    self.error = Some(failure.text);
                }
            }
            Message::AskRename(id) => {
                if let Some(item) = self.item(&id) {
                    self.modal = Some(Modal::Rename {
                        name: item.name.clone(),
                        id,
                    });
                }
            }
            Message::RenameChanged(value) => {
                if let Some(Modal::Rename { name, .. }) = &mut self.modal {
                    *name = value;
                }
            }
            Message::RenameConfirmed => {
                if let Some(Modal::Rename { id, name }) = self.modal.take()
                    && let Some(key) = self.key.clone()
                    && !name.trim().is_empty()
                {
                    match self.store.rename(&key, &id, name.trim()) {
                        Ok(()) => self.reload_items(),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            }
            Message::AskRemove(id) => self.modal = Some(Modal::ConfirmRemoveItem(id)),
            Message::RemoveConfirmed => {
                if let Some(Modal::ConfirmRemoveItem(id)) = self.modal.take()
                    && let Some(key) = self.key.clone()
                {
                    match self.store.remove(&key, &id) {
                        Ok(()) => self.reload_items(),
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            }
            Message::OpenChangePassword => {
                if self.key.is_some() {
                    self.modal = Some(Modal::ChangePassword(PasswordForm::default()));
                }
            }
            Message::PasswordFieldChanged(field, value) => {
                if let Some(Modal::ChangePassword(form)) = &mut self.modal {
                    match field {
                        PasswordField::Old => form.old = value,
                        PasswordField::New => form.new = value,
                        PasswordField::Confirm => form.confirm = value,
                    }
                    form.error = None;
                }
            }
            Message::PasswordSubmitted => {
                if let Some(Modal::ChangePassword(form)) = &mut self.modal
                    && form.can_submit()
                {
                    form.busy = true;
                    let store = self.store.clone();
                    let (old, new) = (form.old.clone(), form.new.clone());
                    return Task::perform(
                        blocking(move || store.change_password(&old, &new)),
                        Message::PasswordUpdated,
                    );
                }
            }
            Message::PasswordUpdated(result) => {
                if let Some(Modal::ChangePassword(form)) = &mut self.modal {
                    form.busy = false;
                    match result {
                        Ok(()) => self.modal = None,
                        Err(failure) if failure.wrong_password => {
                            form.error = Some("Senha atual incorreta".to_string());
                        }
                        Err(failure) => form.error = Some(failure.text),
                    }
                }
            }
            Message::ModalClosed => self.modal = None,
            Message::ErrorDismissed => self.error = None,
            Message::MenuClosed => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![event::listen_with(listen)];
        if self.dirty_since.is_some() {
            subscriptions.push(time::every(AUTOSAVE_POLL).map(|_| Message::Autosave));
        }
        if self.key.is_some() {
            subscriptions.push(time::every(VAULT_POLL).map(|_| Message::VaultTick));
        }
        Subscription::batch(subscriptions)
    }

    fn on_hide(&mut self) {
        self.save_pending();
        self.lock();
    }

    fn context_menu(&self) -> Option<ContextMenu<Message>> {
        self.menu.clone()
    }
}

#[cfg(test)]
mod tests;
