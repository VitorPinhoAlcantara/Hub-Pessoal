mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use hub_core::HubContext;
use hub_core::whatsapp::bridge::{self, BridgeEvent, BridgeHandle};
use hub_core::whatsapp::protocol::Request;
use hub_core::whatsapp::{
    self, Conversation, MESSAGES_LIMIT, Message as ChatMessage, WireChatInfo, WireConversation,
    WireMessage,
};
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::widget::qr_code;
use hub_ui_kit::iced::{Element, Event, Subscription, Task, event, time};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section};

pub const SESSION_ENDED: &str = "sessao_encerrada";
pub const VISIBLE_CONVERSATIONS: usize = 200;
const FLUSH_INTERVAL: Duration = Duration::from_millis(400);
pub const SIDECAR_MISSING: &str = "Não foi possível iniciar";

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
    Bridge(BridgeEvent),
    ConversationsLoaded(Result<Vec<WireConversation>, String>),
    ChatInfoLoaded(Result<Vec<WireChatInfo>, String>),
    ArchivedToggled,
    MediaRequested(String),
    MediaLoaded(String, Result<PathBuf, String>),
    Flush,
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
    MenuClosed,
}

pub struct WhatsApp {
    ctx: HubContext,
    generation: u32,
    connection: Connection,
    handle: Option<BridgeHandle>,
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
    pending: Vec<WireMessage>,
}

impl WhatsApp {
    pub fn new(ctx: HubContext) -> Self {
        let mut section = Self {
            ctx,
            generation: 0,
            connection: Connection::Starting,
            handle: None,
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
            pending: Vec::new(),
        };
        section.reload();
        section
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.connection, Connection::Connected(_)) && self.handle.is_some()
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

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        let touches_selected = self
            .selected
            .as_ref()
            .is_some_and(|jid| pending.iter().any(|wire| &wire.jid == jid));
        let messages: Vec<ChatMessage> = pending.iter().map(whatsapp::wire_to_message).collect();
        if let Err(error) = whatsapp::store_messages(self.ctx.db().conn(), &messages) {
            tracing::warn!(%error, "falha ao gravar mensagens do WhatsApp");
        }
        self.reload_list();
        if touches_selected {
            self.reload_messages();
        }
    }

    fn store(&mut self, wire: &WireMessage) {
        let message = whatsapp::wire_to_message(wire);
        if let Err(error) = whatsapp::store_message(self.ctx.db().conn(), &message) {
            tracing::warn!(%error, "falha ao gravar mensagem do WhatsApp");
        }
    }

    fn request_chat_info(&self) -> Task<Message> {
        let Some(handle) = self.handle.clone() else {
            return Task::none();
        };
        let jids = whatsapp::known_jids(self.ctx.db().conn()).unwrap_or_default();
        if jids.is_empty() {
            return Task::none();
        }
        Task::perform(
            async move { handle.chat_info(jids).await },
            Message::ChatInfoLoaded,
        )
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
        let Some(handle) = self.handle.clone().filter(|_| self.is_connected()) else {
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

    fn set_handle(&mut self, handle: Option<BridgeHandle>) {
        self.ctx.whatsapp.set(handle.clone());
        self.handle = handle;
    }

    fn on_bridge(&mut self, event: BridgeEvent) -> Task<Message> {
        match event {
            BridgeEvent::Ready(handle) => {
                handle.send(Request::Login);
                self.set_handle(Some(handle));
                self.connection = Connection::WaitingQr;
            }
            BridgeEvent::Qr(code) => match qr_code::Data::new(code) {
                Ok(data) => self.connection = Connection::Qr(data),
                Err(error) => self.error = Some(error.to_string()),
            },
            BridgeEvent::Connected(jid) => {
                self.connection = Connection::Connected(jid);
                if let Some(handle) = self.handle.clone() {
                    return Task::perform(
                        async move { handle.conversations().await },
                        Message::ConversationsLoaded,
                    );
                }
            }
            BridgeEvent::Message(wire) => self.pending.push(wire),
            BridgeEvent::Disconnected(reason) => {
                self.connection = if reason == SESSION_ENDED {
                    Connection::WaitingQr
                } else {
                    Connection::Offline(format!("Conexão perdida ({reason})"))
                };
                if reason == SESSION_ENDED
                    && let Some(handle) = &self.handle
                {
                    handle.send(Request::Login);
                }
            }
            BridgeEvent::Archived(jid, archived) => {
                if let Err(error) = whatsapp::set_archived(self.ctx.db().conn(), &jid, archived) {
                    tracing::warn!(%error, "falha ao marcar conversa arquivada");
                }
                self.reload();
            }
            BridgeEvent::Error(error) => self.error = Some(error),
            BridgeEvent::Exited(reason) => {
                self.set_handle(None);
                self.connection = Connection::Offline(reason);
            }
        }
        Task::none()
    }
}

impl Drop for WhatsApp {
    fn drop(&mut self) {
        self.ctx.whatsapp.set(None);
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

fn spawn_bridge(
    key: &(PathBuf, u32),
) -> impl hub_ui_kit::iced::futures::Stream<Item = BridgeEvent> + use<> {
    let dir = key.0.clone();
    bridge::spawn(whatsapp::sidecar_command(&dir), dir)
}

impl Section for WhatsApp {
    type Message = Message;

    fn title(&self) -> &str {
        "WhatsApp"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if !matches!(message, Message::ConversationMenu(_) | Message::Bridge(_)) {
            self.menu = None;
        }
        match message {
            Message::ConversationMenu(jid) => {
                self.menu = Some(ContextMenu::at_cursor(
                    vec![
                        MenuItem::new("Abrir conversa", Message::Selected(jid.clone()))
                            .icon(Icon::WhatsApp),
                        MenuItem::new("Definir apelido", Message::RenameConversation(jid))
                            .icon(Icon::Edit),
                    ],
                    Message::MenuClosed,
                ));
            }
            Message::RenameConversation(jid) => {
                self.selected = Some(jid);
                self.reload();
                self.alias = self.selected().map(|c| c.alias.clone().unwrap_or_default());
            }
            Message::MenuClosed => {}
            Message::Bridge(event) => return self.on_bridge(event),
            Message::ConversationsLoaded(result) => match result {
                Ok(list) => {
                    {
                        let db = self.ctx.db();
                        for conversation in &list {
                            let _ = whatsapp::upsert_conversation(db.conn(), conversation);
                        }
                    }
                    self.reload();
                    return self.request_chat_info();
                }
                Err(error) => self.error = Some(error),
            },
            Message::ChatInfoLoaded(result) => match result {
                Ok(infos) => {
                    if let Err(error) = whatsapp::apply_chat_info(self.ctx.db().conn(), &infos) {
                        self.error = Some(error.to_string());
                    }
                    if let Some(jid) = &self.selected
                        && let Some(phone) = infos
                            .iter()
                            .find(|i| &i.jid == jid && !i.phone.is_empty())
                            .map(|i| i.phone.clone())
                    {
                        self.selected = Some(phone);
                    }
                    self.reload();
                }
                Err(error) => tracing::warn!(%error, "falha ao consultar conversas do WhatsApp"),
            },
            Message::ArchivedToggled => self.show_archived = !self.show_archived,
            Message::Flush => self.flush(),
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
                let (Some(jid), Some(handle)) = (self.selected.clone(), self.handle.clone()) else {
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
                self.generation += 1;
                self.connection = Connection::Starting;
                self.error = None;
            }
            Message::Refresh => self.reload(),
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
        let mut subscriptions = vec![
            Subscription::run_with(
                (self.ctx.paths.data_dir.clone(), self.generation),
                spawn_bridge,
            )
            .map(Message::Bridge),
            event::listen_with(listen),
        ];
        if !self.pending.is_empty() {
            subscriptions.push(time::every(FLUSH_INTERVAL).map(|_| Message::Flush));
        }
        Subscription::batch(subscriptions)
    }

    fn on_hide(&mut self) {
        self.flush();
        self.set_handle(None);
    }
}

#[cfg(test)]
mod tests;
