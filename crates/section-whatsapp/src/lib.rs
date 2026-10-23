mod view;

use std::collections::HashMap;
use std::path::PathBuf;

use hub_core::HubContext;
use hub_core::whatsapp::{
    self, Conversation, MESSAGES_LIMIT, Message as ChatMessage, Status, WireMessage,
};
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::widget::qr_code;
use hub_ui_kit::iced::{Element, Event, Subscription, Task, event};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section};

pub use hub_core::whatsapp::service::SESSION_ENDED;
pub const VISIBLE_CONVERSATIONS: usize = 200;
pub const SIDECAR_MISSING: &str = "Não foi possível iniciar";
pub const SWITCHED_OFF: &str = "Você desligou o WhatsApp. Ele volta a ficar conectado em segundo plano quando você conectar de novo.";

#[derive(Debug, Clone)]
pub enum Media {
    Loading,
    Ready(Handle, PathBuf),
    Failed(String),
}

pub enum Connection {
    Starting,
    WaitingQr,
    Qr(qr_code::Data),
    Connected(String),
    Offline(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    ArchivedToggled,
    MediaRequested(String),
    MediaLoaded(String, Result<PathBuf, String>),
    QueryChanged(String),
    Selected(String),
    DraftChanged(String),
    Send,
    Sent(Result<WireMessage, String>),
    AliasEdit,
    AliasChanged(String),
    AliasSaved,
    AliasCancelled,
    Retry,
    Refresh,
    ErrorDismissed,
    ConversationMenu(String),
    RenameConversation(String),
    ToggleNotify(String),
    SwitchOff,
    MenuClosed,
}

pub struct WhatsApp {
    ctx: HubContext,
    connection: Connection,
    qr: Option<String>,
    conversations: Vec<Conversation>,
    query: String,
    selected: Option<String>,
    messages: Vec<ChatMessage>,
    draft: String,
    sending: bool,
    alias: Option<String>,
    error: Option<String>,
    menu: Option<ContextMenu<Message>>,
    show_archived: bool,
    media: HashMap<String, Media>,
}

impl WhatsApp {
    pub fn new(ctx: HubContext) -> Self {
        let mut section = Self {
            ctx,
            connection: Connection::Starting,
            qr: None,
            conversations: Vec::new(),
            query: String::new(),
            selected: None,
            messages: Vec::new(),
            draft: String::new(),
            sending: false,
            alias: None,
            error: None,
            menu: None,
            show_archived: false,
            media: HashMap::new(),
        };
        section.sync_status();
        section.reload();
        section
    }

    pub fn open_conversation(&mut self, jid: String) {
        self.selected = Some(jid);
        self.alias = None;
        self.reload();
    }

    fn sync_status(&mut self) {
        let status = self.ctx.whatsapp.status();
        self.connection = match status {
            Status::Off => Connection::Offline(SWITCHED_OFF.to_string()),
            Status::Starting => Connection::Starting,
            Status::WaitingQr => Connection::WaitingQr,
            Status::Qr(code) => {
                if self.qr.as_ref() == Some(&code)
                    && let Connection::Qr(_) = self.connection
                {
                    return;
                }
                match qr_code::Data::new(code.clone()) {
                    Ok(data) => {
                        self.qr = Some(code);
                        Connection::Qr(data)
                    }
                    Err(error) => {
                        self.error = Some(error.to_string());
                        Connection::WaitingQr
                    }
                }
            }
            Status::Connected(jid) => Connection::Connected(jid),
            Status::Offline(reason) => Connection::Offline(reason),
        };
        if let Some(error) = self.ctx.whatsapp.take_error() {
            self.error = Some(error);
        }
    }

    pub fn is_running(&self) -> bool {
        self.ctx.whatsapp.is_running()
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn is_connected(&self) -> bool {
        self.ctx.whatsapp.is_connected()
    }

    pub fn conversations(&self) -> Vec<&Conversation> {
        let searching = !self.query.trim().is_empty();
        self.conversations
            .iter()
            .filter(|c| {
                searching
                    || (c.archived == self.show_archived
                        && (c.last_at.is_some() || c.alias.is_some()))
            })
            .filter(|c| c.matches(&self.query))
            .collect()
    }

    pub fn showing_archived(&self) -> bool {
        self.show_archived
    }

    pub fn archived_count(&self) -> usize {
        self.conversations.iter().filter(|c| c.archived).count()
    }

    pub fn media(&self, id: &str) -> Option<&Media> {
        self.media.get(id)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> Option<&Conversation> {
        self.selected
            .as_deref()
            .and_then(|jid| self.conversations.iter().find(|c| c.jid == jid))
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn draft(&self) -> &str {
        &self.draft
    }

    pub fn sending(&self) -> bool {
        self.sending
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn reload(&mut self) {
        self.reload_list();
        self.reload_messages();
    }

    fn reload_list(&mut self) {
        let result = whatsapp::conversations(self.ctx.db().conn());
        match result {
            Ok(list) => self.conversations = list,
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn reload_messages(&mut self) {
        self.messages = match &self.selected {
            Some(jid) => {
                whatsapp::messages(self.ctx.db().conn(), jid, MESSAGES_LIMIT).unwrap_or_default()
            }
            None => Vec::new(),
        };
    }

    fn store(&mut self, wire: &WireMessage) {
        let message = whatsapp::wire_to_message(wire);
        if let Err(error) = whatsapp::store_message(self.ctx.db().conn(), &message) {
            tracing::warn!(%error, "falha ao gravar mensagem do WhatsApp");
        }
    }

    fn request_media(&mut self, id: String) -> Task<Message> {
        match self.media.get(&id) {
            Some(Media::Loading) => return Task::none(),
            Some(Media::Ready(_, path)) => {
                if let Err(error) = self.ctx.platform.open_path(path) {
                    self.error = Some(format!("Não foi possível abrir a imagem: {error}"));
                }
                return Task::none();
            }
            _ => {}
        }
        let Some(media) = self
            .messages
            .iter()
            .find(|m| m.id == id && m.has_media())
            .map(|m| m.media.clone())
        else {
            return Task::none();
        };
        let Some(handle) = self.ctx.whatsapp.get().filter(|_| self.is_connected()) else {
            self.media.insert(
                id,
                Media::Failed("Conecte o WhatsApp para baixar a imagem.".to_string()),
            );
            return Task::none();
        };
        self.media.insert(id.clone(), Media::Loading);
        Task::perform(
            async move {
                let result = handle.download_media(&id, &media).await;
                (id, result)
            },
            |(id, result)| Message::MediaLoaded(id, result),
        )
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
        }) => Some(Message::AliasCancelled),
        _ => None,
    }
}

impl Section for WhatsApp {
    type Message = Message;

    fn title(&self) -> &str {
        "WhatsApp"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if !matches!(message, Message::ConversationMenu(_) | Message::Refresh) {
            self.menu = None;
        }
        match message {
            Message::ConversationMenu(jid) => {
                let mut items = vec![
                    MenuItem::new("Abrir conversa", Message::Selected(jid.clone()))
                        .icon(Icon::WhatsApp),
                    MenuItem::new("Definir apelido", Message::RenameConversation(jid.clone()))
                        .icon(Icon::Edit),
                ];
                if whatsapp::is_group_jid(&jid) {
                    let notify = self.conversations.iter().any(|c| c.jid == jid && c.notify);
                    let label = if notify {
                        "Parar de notificar este grupo"
                    } else {
                        "Notificar mensagens deste grupo"
                    };
                    items.push(MenuItem::new(label, Message::ToggleNotify(jid)).icon(Icon::Bell));
                }
                self.menu = Some(ContextMenu::at_cursor(items, Message::MenuClosed));
            }
            Message::ToggleNotify(jid) => {
                let notify = !self.conversations.iter().any(|c| c.jid == jid && c.notify);
                if let Err(error) = whatsapp::set_notify(self.ctx.db().conn(), &jid, notify) {
                    self.error = Some(error.to_string());
                }
                self.reload_list();
            }
            Message::SwitchOff => {
                self.ctx.whatsapp.stop(&self.ctx);
                self.sync_status();
            }
            Message::RenameConversation(jid) => {
                self.selected = Some(jid);
                self.reload();
                self.alias = self.selected().map(|c| c.alias.clone().unwrap_or_default());
            }
            Message::MenuClosed => {}
            Message::ArchivedToggled => self.show_archived = !self.show_archived,
            Message::MediaRequested(id) => return self.request_media(id),
            Message::MediaLoaded(id, result) => {
                let state = match result {
                    Ok(path) => Media::Ready(Handle::from_path(&path), path),
                    Err(error) => Media::Failed(error),
                };
                self.media.insert(id, state);
            }
            Message::QueryChanged(query) => self.query = query,
            Message::Selected(jid) => {
                if self.selected.as_ref() == Some(&jid) {
                    return Task::none();
                }
                self.selected = Some(jid);
                self.alias = None;
                self.reload_messages();
            }
            Message::DraftChanged(draft) => self.draft = draft,
            Message::Send => {
                let text = self.draft.trim().to_string();
                let (Some(jid), Some(handle)) = (self.selected.clone(), self.ctx.whatsapp.get())
                else {
                    return Task::none();
                };
                if text.is_empty() || self.sending || !self.is_connected() {
                    return Task::none();
                }
                self.sending = true;
                return Task::perform(
                    async move { handle.send_text(&jid, &text).await },
                    Message::Sent,
                );
            }
            Message::Sent(result) => {
                self.sending = false;
                match result {
                    Ok(wire) => {
                        self.draft.clear();
                        self.store(&wire);
                        self.reload();
                    }
                    Err(error) => self.error = Some(format!("Não foi possível enviar: {error}")),
                }
            }
            Message::AliasEdit => {
                self.alias = self.selected().map(|c| c.alias.clone().unwrap_or_default());
            }
            Message::AliasChanged(text) => {
                if self.alias.is_some() {
                    self.alias = Some(text);
                }
            }
            Message::AliasSaved => {
                if let (Some(jid), Some(alias)) = (self.selected.clone(), self.alias.take()) {
                    if let Err(error) = whatsapp::set_alias(self.ctx.db().conn(), &jid, &alias) {
                        self.error = Some(error.to_string());
                    }
                    self.reload();
                }
            }
            Message::AliasCancelled => self.alias = None,
            Message::Retry => {
                self.error = None;
                self.ctx.whatsapp.restart(&self.ctx);
                self.sync_status();
            }
            Message::Refresh => {
                self.sync_status();
                self.reload();
            }
            Message::ErrorDismissed => self.error = None,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn context_menu(&self) -> Option<ContextMenu<Message>> {
        self.menu.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        event::listen_with(listen)
    }
}

#[cfg(test)]
mod tests;
