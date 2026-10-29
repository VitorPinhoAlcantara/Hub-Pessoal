mod view;

use std::collections::HashMap;
use std::path::PathBuf;

use hub_core::HubContext;
use hub_core::whatsapp::protocol::WireParticipant;
use hub_core::whatsapp::{
    self, Conversation, MESSAGES_LIMIT, Message as ChatMessage, Status, WireMessage,
};
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::widget::qr_code;
use hub_ui_kit::iced::{Element, Event, Point, Subscription, Task, event, mouse};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section};

pub use hub_core::whatsapp::service::SESSION_ENDED;
pub const VISIBLE_CONVERSATIONS: usize = 200;
pub const DRAG_THRESHOLD: f32 = 6.0;
pub const EVERYONE: &str = "todos";
pub const MENTION_SUGGESTIONS: usize = 6;
pub const DRAFT_INPUT: &str = "whatsapp-mensagem";

pub fn apply_mentions(
    text: &str,
    mentions: &[(String, String)],
    everyone: &[String],
) -> (String, Vec<String>) {
    let mut sorted: Vec<&(String, String)> = mentions.iter().collect();
    sorted.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    let mut out = text.to_string();
    let mut jids = Vec::new();
    for (name, jid) in sorted {
        let tag = format!("@{name}");
        if out.contains(&tag) {
            let user = jid.split('@').next().unwrap_or(jid);
            out = out.replace(&tag, &format!("@{user}"));
            jids.push(jid.clone());
        }
    }
    let lower = text.to_lowercase();
    if lower.contains(&format!("@{EVERYONE}")) || lower.contains("@all") {
        jids.extend(everyone.iter().cloned());
    }
    jids.sort();
    jids.dedup();
    (out, jids)
}
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
    AttachFiles,
    ParticipantsLoaded(String, Result<Vec<WireParticipant>, String>),
    MentionPicked(String, Option<String>),
    FilesPicked(Vec<PathBuf>),
    FilesSent(Vec<Result<WireMessage, String>>),
    TogglePinned(String),
    ConversationPressed(String),
    ConversationHovered(String),
    ConversationLeft(String),
    DragMoved(Point),
    DragReleased,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub jid: String,
    pub start: Point,
    pub active: bool,
}

pub fn reorder_pins(pins: &[String], dragged: &str, target: &str) -> Option<Vec<String>> {
    if dragged == target {
        return None;
    }
    let mut order: Vec<String> = pins.iter().filter(|j| *j != dragged).cloned().collect();
    match pins.iter().position(|j| j == target) {
        Some(to) => {
            let mut index = order.iter().position(|j| j == target)?;
            if pins.iter().position(|j| j == dragged).is_some_and(|from| from < to) {
                index += 1;
            }
            order.insert(index, dragged.to_string());
            Some(order)
        }
        None if order.len() < pins.len() => Some(order),
        None => None,
    }
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
    drag: Option<Drag>,
    hovered: Option<String>,
    participants: Option<(String, Vec<WireParticipant>)>,
    mentions: Vec<(String, String)>,
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
            drag: None,
            hovered: None,
            participants: None,
            mentions: Vec::new(),
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

    fn mention_query(&self) -> Option<String> {
        let token = self.draft.rsplit(char::is_whitespace).next()?;
        token.strip_prefix('@').map(str::to_lowercase)
    }

    fn group_participants(&self) -> Option<&[WireParticipant]> {
        let selected = self.selected.as_deref()?;
        match &self.participants {
            Some((jid, list)) if jid == selected => Some(list),
            _ => None,
        }
    }

    pub fn mention_suggestions(&self) -> Vec<(String, Option<String>)> {
        let Some(query) = self.mention_query() else {
            return Vec::new();
        };
        let Some(participants) = self.group_participants() else {
            return Vec::new();
        };
        let mut list = Vec::new();
        if EVERYONE.starts_with(&query) {
            list.push((EVERYONE.to_string(), None));
        }
        list.extend(
            participants
                .iter()
                .filter(|p| p.nome.to_lowercase().contains(&query))
                .take(MENTION_SUGGESTIONS)
                .map(|p| (p.nome.clone(), Some(p.jid.clone()))),
        );
        list
    }

    pub fn drop_target(&self, jid: &str) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|d| d.active && d.jid != jid && self.hovered.as_deref() == Some(jid))
    }

    fn pinned(&self) -> Vec<String> {
        self.conversations
            .iter()
            .filter(|c| c.pinned)
            .map(|c| c.jid.clone())
            .collect()
    }

    fn save_pins(&mut self, pins: &[String]) {
        if let Err(error) = whatsapp::set_pinned_order(self.ctx.db().conn(), pins) {
            self.error = Some(error.to_string());
        }
        self.reload_list();
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

fn drag_events(
    event: Event,
    _status: event::Status,
    _window: hub_ui_kit::iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::DragMoved(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::DragReleased)
        }
        _ => None,
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
        if !matches!(
            message,
            Message::ConversationMenu(_)
                | Message::Refresh
                | Message::ConversationHovered(_)
                | Message::ConversationLeft(_)
                | Message::DragMoved(_)
        ) {
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
                let pinned = self.conversations.iter().any(|c| c.jid == jid && c.pinned);
                items.push(
                    MenuItem::new(
                        if pinned { "Desafixar" } else { "Fixar no topo" },
                        Message::TogglePinned(jid.clone()),
                    )
                    .icon(Icon::Flag),
                );
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
            Message::TogglePinned(jid) => {
                let mut pins = self.pinned();
                if pins.contains(&jid) {
                    pins.retain(|j| *j != jid);
                } else {
                    pins.push(jid);
                }
                self.save_pins(&pins);
            }
            Message::ConversationPressed(jid) => {
                self.drag = Some(Drag {
                    jid,
                    start: hub_ui_kit::cursor::position(),
                    active: false,
                });
            }
            Message::ConversationHovered(jid) => self.hovered = Some(jid),
            Message::ConversationLeft(jid) => {
                if self.hovered.as_ref() == Some(&jid) {
                    self.hovered = None;
                }
            }
            Message::DragMoved(position) => {
                if let Some(drag) = &mut self.drag
                    && !drag.active
                    && drag.start.distance(position) > DRAG_THRESHOLD
                {
                    drag.active = true;
                }
            }
            Message::DragReleased => {
                if let (Some(drag), Some(target)) = (self.drag.take(), self.hovered.clone())
                    && drag.active
                    && let Some(pins) = reorder_pins(&self.pinned(), &drag.jid, &target)
                {
                    self.save_pins(&pins);
                }
            }
            Message::AttachFiles => {
                return Task::perform(hub_ui_kit::dialog::pick_files(), Message::FilesPicked);
            }
            Message::FilesPicked(paths) => {
                let (Some(jid), Some(handle)) = (self.selected.clone(), self.ctx.whatsapp.get())
                else {
                    return Task::none();
                };
                if paths.is_empty() || self.sending || !self.is_connected() {
                    return Task::none();
                }
                self.sending = true;
                let caption = self.draft.trim().to_string();
                return Task::perform(
                    async move {
                        let mut results = Vec::new();
                        for (index, path) in paths.iter().enumerate() {
                            let caption = if index == 0 { caption.as_str() } else { "" };
                            results.push(handle.send_file(&jid, path, caption).await);
                        }
                        results
                    },
                    Message::FilesSent,
                );
            }
            Message::FilesSent(results) => {
                self.sending = false;
                if results.first().is_some_and(Result::is_ok) {
                    self.draft.clear();
                }
                let errors: Vec<String> = results
                    .iter()
                    .filter_map(|r| r.as_ref().err().cloned())
                    .collect();
                for wire in results.iter().flatten() {
                    self.store(wire);
                }
                if !errors.is_empty() {
                    self.error = Some(errors.join("\n"));
                }
                self.reload();
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
                self.mentions.clear();
                self.reload_messages();
            }
            Message::DraftChanged(draft) => {
                self.draft = draft;
                let wants_group = self
                    .selected
                    .clone()
                    .filter(|jid| whatsapp::is_group_jid(jid))
                    .filter(|_| self.mention_query().is_some());
                if let (Some(jid), Some(handle)) = (wants_group, self.ctx.whatsapp.get())
                    && self.participants.as_ref().is_none_or(|(loaded, _)| *loaded != jid)
                {
                    self.participants = Some((jid.clone(), Vec::new()));
                    return Task::perform(
                        async move {
                            let result = handle.participants(&jid).await;
                            (jid, result)
                        },
                        |(jid, result)| Message::ParticipantsLoaded(jid, result),
                    );
                }
            }
            Message::ParticipantsLoaded(jid, result) => match result {
                Ok(mut list) => {
                    list.sort_by_key(|p| p.nome.to_lowercase());
                    self.participants = Some((jid, list));
                }
                Err(error) => tracing::warn!(%error, "falha ao listar participantes"),
            },
            Message::MentionPicked(name, jid) => {
                let cut = self
                    .draft
                    .rfind(char::is_whitespace)
                    .map_or(0, |index| index + 1);
                self.draft.truncate(cut);
                self.draft.push_str(&format!("@{name} "));
                if let Some(jid) = jid {
                    self.mentions.push((name, jid));
                }
                let id = hub_ui_kit::iced::widget::Id::new(DRAFT_INPUT);
                return Task::batch([
                    hub_ui_kit::iced::widget::operation::focus(id.clone()),
                    hub_ui_kit::iced::widget::operation::move_cursor_to_end(id),
                ]);
            }
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
                let everyone: Vec<String> = self
                    .group_participants()
                    .map(|list| list.iter().map(|p| p.jid.clone()).collect())
                    .unwrap_or_default();
                let (text, mentions) = apply_mentions(&text, &self.mentions, &everyone);
                return Task::perform(
                    async move { handle.send_with_mentions(&jid, &text, mentions).await },
                    Message::Sent,
                );
            }
            Message::Sent(result) => {
                self.sending = false;
                match result {
                    Ok(mut wire) => {
                        wire.texto = self.draft.trim().to_string();
                        self.draft.clear();
                        self.mentions.clear();
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
        if self.drag.is_some() {
            Subscription::batch([event::listen_with(listen), event::listen_with(drag_events)])
        } else {
            event::listen_with(listen)
        }
    }
}

#[cfg(test)]
mod tests;
