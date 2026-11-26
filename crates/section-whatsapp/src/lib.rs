mod actions;
mod chat;
mod composer;
mod emoji;
mod media;
mod model;
mod photos;
mod stickers;
mod view;
mod voice;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use hub_core::HubContext;
use hub_core::chrono::NaiveDateTime;
use hub_core::whatsapp::protocol::WireParticipant;
use hub_core::whatsapp::{
    self, Conversation, Message as ChatMessage, Reaction, Status, WireEdit, WireMessage,
    WireReaction, WireRevoke,
};
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::widget::qr_code;
use hub_ui_kit::iced::{Element, Event, Point, Subscription, Task, event, mouse};
use hub_ui_kit::{ContextMenu, Section};

pub use hub_core::whatsapp::service::SESSION_ENDED;
pub use model::{
    Dialog, Draft, DrawerTab, Media, MediaFile, Outgoing, Pasted, Voice, can_edit, is_dangerous,
    merge_messages, toggle_destination, visible_range,
};

use model::*;

pub const VISIBLE_CONVERSATIONS: usize = 200;
pub const DRAG_THRESHOLD: f32 = 6.0;
pub const EVERYONE: &str = "todos";
pub const MENTION_SUGGESTIONS: usize = 6;
pub const DRAFT_INPUT: &str = "whatsapp-mensagem";
pub const SEARCH_INPUT: &str = "whatsapp-busca";
pub const SWITCHED_OFF: &str = "Você desligou o WhatsApp. Ele volta a ficar conectado em segundo plano quando você conectar de novo.";

static SAVED_DRAFTS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

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
    QueryChanged(String),
    GlobalHits(u64, Result<Vec<ChatMessage>, String>),
    Selected(String),
    DraftChanged(String),
    Send,
    Sent(Outgoing, Vec<Result<WireMessage, String>>),
    AliasEdit,
    AliasChanged(String),
    AliasSaved,
    AliasCancelled,
    Retry,
    Refresh,
    ErrorDismissed,
    StatusHovered(bool),
    ConversationMenu(String),
    RenameConversation(String),
    ToggleNotify(String),
    SwitchOff,
    MenuClosed,
    AttachFiles,
    ParticipantsLoaded(String, Result<Vec<WireParticipant>, String>),
    MentionPicked(String, Option<String>),
    FilesPicked(Vec<PathBuf>),
    TogglePinned(String),
    ConversationPressed(String),
    ConversationHovered(String),
    ConversationLeft(String),
    DragMoved(Point),
    DragReleased,
    MediaRequested(String),
    MediaOpen(String),
    MediaReveal(String),
    MediaLoaded(String, Result<MediaFile, String>),
    PhotoReady(String, Option<Handle>),
    MessageMenu(String),
    ReplyTo(String),
    CancelReply,
    ReactionPicker(String),
    ReactWith(String, String),
    Reacted(Result<WireReaction, String>),
    CopyText(String),
    ForwardOpen(String),
    ForwardQuery(String),
    ForwardToggle(String),
    ForwardConfirm,
    Forwarded(Result<Vec<Result<WireMessage, String>>, String>),
    EditStart(String),
    Edited(Outgoing, Result<WireEdit, String>),
    RevokeAsk(String),
    RevokeConfirm(String),
    Revoked(Result<WireRevoke, String>),
    DeleteLocalAsk(String),
    DeleteLocalConfirm(String),
    DialogClosed,
    EmojiToggle,
    EmojiQuery(String),
    EmojiCategory(usize),
    EmojiPicked(String),
    StickerTab,
    StickerSectionPicked(whatsapp::StickerSection),
    StickersLoaded(
        whatsapp::StickerSection,
        Result<Vec<whatsapp::Sticker>, String>,
    ),
    StickerPreview(String, Option<Handle>),
    SendSticker(String, Vec<u8>),
    ToggleFavorite(String, Vec<u8>),
    FavoriteSaved(String, Vec<u8>, bool, Result<(), String>),
    StickerFile,
    StickerFilePicked(Option<PathBuf>),
    VideoThumbs(Vec<(String, Option<Handle>)>),
    PasteShortcut,
    PasteRead(String, Option<Pasted>),
    RemoveImage,
    RecordStart,
    RecordCancel,
    RecordStop,
    VoiceReady(String, Result<Voice, String>),
    PlaybackReady(String, Result<voice::ReadyPlayback, String>),
    RemoveVoice,
    Tick,
    AudioToggle(String),
    AudioSeeking(String, f32),
    CancelEdit,
    AudioSeekCommit,
    AudioSpeed,
    LoadOlder,
    OlderLoaded(String, Result<Vec<ChatMessage>, String>),
    HistoryRequested(String, Result<(), String>),
    HistoryTimeout(String, u64),
    JumpTo(String, String),
    JumpLoaded(String, String, Result<Vec<ChatMessage>, String>),
    BackToRecent,
    SearchToggle,
    SearchChanged(String),
    SearchResults(u64, Result<Vec<ChatMessage>, String>),
    SearchNext,
    SearchPrevious,
    DrawerToggle,
    DrawerTab(DrawerTab),
    DrawerLoaded(String, Result<Vec<ChatMessage>, String>),
    Escape,
}

fn is_passive(message: &Message) -> bool {
    matches!(
        message,
        Message::ConversationMenu(_)
            | Message::MessageMenu(_)
            | Message::Refresh
            | Message::ConversationHovered(_)
            | Message::ConversationLeft(_)
            | Message::StatusHovered(_)
            | Message::DragMoved(_)
            | Message::Tick
            | Message::MediaLoaded(..)
            | Message::PhotoReady(..)
            | Message::StickersLoaded(..)
            | Message::FavoriteSaved(..)
            | Message::StickerPreview(..)
            | Message::VideoThumbs(_)
            | Message::GlobalHits(..)
            | Message::SearchResults(..)
            | Message::OlderLoaded(..)
            | Message::JumpLoaded(..)
            | Message::HistoryRequested(..)
            | Message::HistoryTimeout(..)
            | Message::DrawerLoaded(..)
            | Message::ParticipantsLoaded(..)
            | Message::PasteRead(..)
            | Message::VoiceReady(..)
            | Message::PlaybackReady(..)
            | Message::Reacted(_)
            | Message::Edited(..)
            | Message::Revoked(_)
            | Message::Forwarded(_)
            | Message::Sent(..)
    )
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
            if pins
                .iter()
                .position(|j| j == dragged)
                .is_some_and(|from| from < to)
            {
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
    reactions: HashMap<String, Vec<Reaction>>,
    view_end: Option<usize>,
    shown: usize,
    highlight: Option<String>,
    drafts: HashMap<String, Draft>,
    sending: HashSet<String>,
    alias: Option<String>,
    error: Option<String>,
    menu: Option<ContextMenu<Message>>,
    show_archived: bool,
    media: HashMap<String, Media>,
    preview_order: Vec<String>,
    intents: HashMap<String, Intent>,
    photos: HashMap<String, Photo>,
    drag: Option<Drag>,
    hovered: Option<String>,
    participants: Option<(String, Vec<WireParticipant>)>,
    dialog: Option<Dialog>,
    emoji_open: bool,
    emoji: EmojiState,
    recording: Option<voice::Recording>,
    player: Option<voice::Player>,
    loading_audio: Option<String>,
    speed: usize,
    history_wait: Option<HistoryWait>,
    history_notice: Option<String>,
    history_generation: u64,
    search: Option<SearchState>,
    search_seq: u64,
    global_hits: Vec<ChatMessage>,
    global_seq: u64,
    drawer: Option<Drawer>,
    status_hover: bool,
    video_thumbs: HashMap<String, Option<Handle>>,
    stickers: Option<Vec<whatsapp::Sticker>>,
    favorites: HashSet<String>,
    sticker_previews: HashMap<String, Option<Handle>>,
    sticker_failed: HashSet<String>,
}

impl WhatsApp {
    pub fn new(ctx: HubContext) -> Self {
        let saved = SAVED_DRAFTS
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default();
        let drafts = saved
            .into_iter()
            .map(|(jid, text)| {
                (
                    jid,
                    Draft {
                        text,
                        ..Draft::default()
                    },
                )
            })
            .collect();
        let mut section = Self {
            ctx,
            connection: Connection::Starting,
            qr: None,
            conversations: Vec::new(),
            query: String::new(),
            selected: None,
            messages: Vec::new(),
            reactions: HashMap::new(),
            view_end: None,
            shown: WINDOW_SIZE,
            highlight: None,
            drafts,
            sending: HashSet::new(),
            alias: None,
            error: None,
            menu: None,
            show_archived: false,
            media: HashMap::new(),
            preview_order: Vec::new(),
            intents: HashMap::new(),
            photos: HashMap::new(),
            drag: None,
            hovered: None,
            participants: None,
            dialog: None,
            emoji_open: false,
            emoji: EmojiState::default(),
            recording: None,
            player: None,
            loading_audio: None,
            speed: 0,
            history_wait: None,
            history_notice: None,
            history_generation: 0,
            search: None,
            search_seq: 0,
            global_hits: Vec::new(),
            global_seq: 0,
            drawer: None,
            status_hover: false,
            video_thumbs: HashMap::new(),
            stickers: None,
            favorites: HashSet::new(),
            sticker_previews: HashMap::new(),
            sticker_failed: HashSet::new(),
        };
        section.sync_status();
        section.reload_list();
        section.reload_favorites();
        section
    }

    pub fn open_conversation(&mut self, jid: String) -> Task<Message> {
        self.select(jid)
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
        self.current_draft().map_or("", |d| d.text.as_str())
    }

    pub fn sending(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|jid| self.sending.contains(jid))
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    fn mention_query(&self) -> Option<String> {
        let token = self.draft().rsplit(char::is_whitespace).next()?;
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

    fn store(&mut self, wire: &WireMessage) {
        let message = whatsapp::wire_to_message(wire);
        if let Err(error) = whatsapp::store_message(self.ctx.db().conn(), &message) {
            tracing::warn!(%error, "falha ao gravar mensagem do WhatsApp");
        }
    }

    fn pump(&mut self) -> Task<Message> {
        Task::batch([
            self.pump_media(),
            self.pump_photos(),
            self.pump_video_thumbs(),
        ])
    }

    fn connected_handle(&self) -> Option<whatsapp::BridgeHandle> {
        self.ctx.whatsapp.get().filter(|_| self.is_connected())
    }

    fn needs_connection(&mut self) -> Option<whatsapp::BridgeHandle> {
        let handle = self.connected_handle();
        if handle.is_none() {
            self.error = Some("Conecte o WhatsApp para fazer isso.".to_string());
        }
        handle
    }

    fn find_message(&self, id: &str) -> Option<&ChatMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    fn now(&self) -> NaiveDateTime {
        self.ctx.clock.now()
    }

    fn focus_input() -> Task<Message> {
        hub_ui_kit::iced::widget::operation::focus(hub_ui_kit::iced::widget::Id::new(DRAFT_INPUT))
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
        }) => Some(Message::Escape),
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character(character),
            modifiers,
            ..
        }) if modifiers.command() && character.as_str().eq_ignore_ascii_case("v") => {
            Some(Message::PasteShortcut)
        }
        _ => None,
    }
}

impl Section for WhatsApp {
    type Message = Message;

    fn title(&self) -> &str {
        "WhatsApp"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if !is_passive(&message) {
            self.menu = None;
        }
        match message {
            Message::ConversationMenu(jid) => self.conversation_menu(jid),
            Message::TogglePinned(jid) => {
                let mut pins = self.pinned();
                if pins.contains(&jid) {
                    pins.retain(|j| *j != jid);
                } else {
                    pins.push(jid);
                }
                self.save_pins(&pins);
                Task::none()
            }
            Message::ConversationPressed(jid) => {
                self.drag = Some(Drag {
                    jid,
                    start: hub_ui_kit::cursor::position(),
                    active: false,
                });
                Task::none()
            }
            Message::ConversationHovered(jid) => {
                self.hovered = Some(jid);
                Task::none()
            }
            Message::ConversationLeft(jid) => {
                if self.hovered.as_ref() == Some(&jid) {
                    self.hovered = None;
                }
                Task::none()
            }
            Message::DragMoved(position) => {
                if let Some(drag) = &mut self.drag
                    && !drag.active
                    && drag.start.distance(position) > DRAG_THRESHOLD
                {
                    drag.active = true;
                }
                Task::none()
            }
            Message::DragReleased => {
                let Some(drag) = self.drag.take() else {
                    return Task::none();
                };
                if !drag.active {
                    return self.select(drag.jid);
                }
                if let Some(target) = self.hovered.clone()
                    && let Some(pins) = reorder_pins(&self.pinned(), &drag.jid, &target)
                {
                    self.save_pins(&pins);
                }
                Task::none()
            }
            Message::AttachFiles => {
                Task::perform(hub_ui_kit::dialog::pick_files(), Message::FilesPicked)
            }
            Message::FilesPicked(paths) => self.send_files(paths),
            Message::ToggleNotify(jid) => {
                let notify = !self.conversations.iter().any(|c| c.jid == jid && c.notify);
                if let Err(error) = whatsapp::set_notify(self.ctx.db().conn(), &jid, notify) {
                    self.error = Some(error.to_string());
                }
                self.reload_list();
                Task::none()
            }
            Message::SwitchOff => {
                self.discard_recording();
                self.ctx.whatsapp.stop(&self.ctx);
                self.sync_status();
                Task::none()
            }
            Message::RenameConversation(jid) => {
                let task = self.select(jid);
                self.alias = self.selected().map(|c| c.alias.clone().unwrap_or_default());
                task
            }
            Message::MenuClosed => Task::none(),
            Message::ArchivedToggled => {
                self.show_archived = !self.show_archived;
                self.pump()
            }
            Message::MediaRequested(id) => self.request_media(id, Intent::Show),
            Message::MediaOpen(id) => self.request_media(id, Intent::Open),
            Message::MediaReveal(id) => {
                self.reveal_media(&id);
                Task::none()
            }
            Message::MediaLoaded(id, result) => self.media_loaded(id, result),
            Message::PhotoReady(jid, handle) => {
                self.photos
                    .insert(jid, handle.map_or(Photo::Missing, Photo::Ready));
                self.pump_photos()
            }
            Message::QueryChanged(query) => self.set_query(query),
            Message::GlobalHits(seq, result) => {
                if seq == self.global_seq {
                    self.global_hits = result.unwrap_or_default();
                }
                Task::none()
            }
            Message::Selected(jid) => self.select(jid),
            Message::DraftChanged(text) => self.draft_changed(text),
            Message::ParticipantsLoaded(jid, result) => {
                match result {
                    Ok(mut list) => {
                        list.sort_by_key(|p| p.nome.to_lowercase());
                        self.participants = Some((jid, list));
                    }
                    Err(error) => tracing::warn!(%error, "falha ao listar participantes"),
                }
                Task::none()
            }
            Message::MentionPicked(name, jid) => self.mention_picked(name, jid),
            Message::Send => self.send(),
            Message::Sent(out, results) => self.sent(out, results),
            Message::AliasEdit => {
                self.alias = self.selected().map(|c| c.alias.clone().unwrap_or_default());
                Task::none()
            }
            Message::AliasChanged(text) => {
                if self.alias.is_some() {
                    self.alias = Some(text);
                }
                Task::none()
            }
            Message::AliasSaved => {
                if let (Some(jid), Some(alias)) = (self.selected.clone(), self.alias.take()) {
                    if let Err(error) = whatsapp::set_alias(self.ctx.db().conn(), &jid, &alias) {
                        self.error = Some(error.to_string());
                    }
                    self.reload_list();
                    self.refresh_messages();
                }
                Task::none()
            }
            Message::AliasCancelled => {
                self.alias = None;
                Task::none()
            }
            Message::Retry => {
                self.error = None;
                self.ctx.whatsapp.restart(&self.ctx);
                self.sync_status();
                Task::none()
            }
            Message::Refresh => self.refresh(),
            Message::StatusHovered(hover) => {
                self.status_hover = hover;
                Task::none()
            }
            Message::ErrorDismissed => {
                self.error = None;
                Task::none()
            }
            Message::MessageMenu(id) => {
                self.message_menu(id);
                Task::none()
            }
            Message::ReplyTo(id) => self.reply_to(id),
            Message::CancelReply => {
                if let Some(draft) = self.draft_mut() {
                    draft.reply = None;
                }
                Task::none()
            }
            Message::ReactionPicker(id) => {
                self.emoji = EmojiState::default();
                self.dialog = Some(Dialog::React(id));
                Task::none()
            }
            Message::ReactWith(id, emoji) => self.react(id, emoji),
            Message::Reacted(result) => self.reacted(result),
            Message::CopyText(id) => self.copy_text(&id),
            Message::ForwardOpen(id) => {
                self.forward_open(id);
                Task::none()
            }
            Message::ForwardQuery(query) => {
                if let Some(Dialog::Forward(state)) = &mut self.dialog {
                    state.query = query;
                }
                Task::none()
            }
            Message::ForwardToggle(jid) => {
                if let Some(Dialog::Forward(state)) = &mut self.dialog {
                    toggle_destination(&mut state.selected, &jid);
                }
                Task::none()
            }
            Message::ForwardConfirm => self.forward_confirm(),
            Message::Forwarded(result) => self.forwarded(result),
            Message::EditStart(id) => self.edit_start(id),
            Message::Edited(out, result) => self.edited(out, result),
            Message::RevokeAsk(id) => {
                self.dialog = Some(Dialog::ConfirmRevoke(id));
                Task::none()
            }
            Message::RevokeConfirm(id) => self.revoke_confirm(id),
            Message::Revoked(result) => self.revoked(result),
            Message::DeleteLocalAsk(id) => {
                self.dialog = Some(Dialog::ConfirmDeleteLocal(id));
                Task::none()
            }
            Message::DeleteLocalConfirm(id) => {
                self.dialog = None;
                self.delete_local(&id);
                Task::none()
            }
            Message::DialogClosed => {
                self.dialog = None;
                Task::none()
            }
            Message::EmojiToggle => {
                self.emoji_open = !self.emoji_open;
                self.emoji = EmojiState::default();
                Task::none()
            }
            Message::EmojiQuery(query) => {
                self.emoji.query = query;
                Task::none()
            }
            Message::EmojiCategory(index) => {
                self.emoji.stickers = false;
                self.emoji.category = index;
                self.emoji.query.clear();
                Task::none()
            }
            Message::EmojiPicked(emoji) => self.emoji_picked(emoji),
            Message::StickerTab => self.open_stickers(),
            Message::StickerSectionPicked(section) => self.pick_sticker_section(section),
            Message::StickersLoaded(section, result) => self.stickers_loaded(section, result),
            Message::ToggleFavorite(hash, action) => self.toggle_favorite(hash, action),
            Message::FavoriteSaved(hash, action, favorite, result) => {
                self.favorite_saved(hash, action, favorite, result)
            }
            Message::StickerPreview(id, handle) => {
                if handle.is_none() {
                    self.sticker_failed.insert(id.clone());
                }
                self.sticker_previews.insert(id, handle);
                Task::none()
            }
            Message::SendSticker(hash, action) => self.send_sticker(hash, action),
            Message::StickerFile => Task::perform(
                hub_ui_kit::dialog::pick_file("Imagem para a figurinha"),
                Message::StickerFilePicked,
            ),
            Message::StickerFilePicked(path) => self.send_sticker_file(path),
            Message::VideoThumbs(list) => {
                self.video_thumbs.extend(list);
                Task::none()
            }
            Message::PasteShortcut => self.paste_shortcut(),
            Message::PasteRead(jid, pasted) => self.paste_read(jid, pasted),
            Message::RemoveImage => {
                if let Some(draft) = self.draft_mut() {
                    draft.image = None;
                }
                Task::none()
            }
            Message::RecordStart => {
                self.start_recording();
                Task::none()
            }
            Message::RecordCancel => {
                self.discard_recording();
                Task::none()
            }
            Message::RecordStop => self.stop_recording(),
            Message::VoiceReady(jid, result) => self.voice_ready(jid, result),
            Message::PlaybackReady(id, result) => {
                self.playback_ready(id, result);
                Task::none()
            }
            Message::RemoveVoice => {
                self.remove_voice();
                Task::none()
            }
            Message::Tick => {
                self.tick();
                Task::none()
            }
            Message::AudioToggle(id) => self.toggle_audio(id),
            Message::AudioSeeking(id, fraction) => {
                if let Some(player) = &mut self.player
                    && player.id == id
                {
                    player.seeking = Some(fraction);
                }
                Task::none()
            }
            Message::CancelEdit => {
                self.cancel_edit();
                Task::none()
            }
            Message::AudioSeekCommit => {
                self.commit_seek();
                Task::none()
            }
            Message::AudioSpeed => {
                self.cycle_speed();
                Task::none()
            }
            Message::LoadOlder => self.load_older(),
            Message::OlderLoaded(jid, result) => self.older_loaded(jid, result),
            Message::HistoryRequested(jid, result) => self.history_requested(jid, result),
            Message::HistoryTimeout(jid, generation) => {
                self.history_timeout(&jid, generation);
                Task::none()
            }
            Message::JumpTo(jid, id) => self.jump_to(jid, id),
            Message::JumpLoaded(jid, id, result) => self.jump_loaded(jid, id, result),
            Message::BackToRecent => self.back_to_recent(),
            Message::SearchToggle => self.search_toggle(),
            Message::SearchChanged(query) => self.search_changed(query),
            Message::SearchResults(seq, result) => self.search_results(seq, result),
            Message::SearchNext => self.search_step(true),
            Message::SearchPrevious => self.search_step(false),
            Message::DrawerToggle => self.drawer_toggle(),
            Message::DrawerTab(tab) => {
                if let Some(drawer) = &mut self.drawer {
                    drawer.tab = tab;
                }
                Task::none()
            }
            Message::DrawerLoaded(jid, result) => {
                self.drawer_loaded(&jid, result);
                Task::none()
            }
            Message::Escape => self.escape(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn context_menu(&self) -> Option<ContextMenu<Message>> {
        self.menu.clone()
    }

    fn on_hide(&mut self) {
        self.discard_recording();
        self.player = None;
        self.status_hover = false;
        self.save_drafts();
        self.drop_hidden_previews();
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![event::listen_with(listen)];
        if self.drag.is_some() {
            subscriptions.push(event::listen_with(drag_events));
        }
        if self.is_ticking() {
            subscriptions.push(
                hub_ui_kit::iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick),
            );
        }
        Subscription::batch(subscriptions)
    }
}

impl WhatsApp {
    fn conversation_menu(&mut self, jid: String) -> Task<Message> {
        use hub_ui_kit::{Icon, MenuItem};
        let mut items = vec![
            MenuItem::new("Abrir conversa", Message::Selected(jid.clone())).icon(Icon::WhatsApp),
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
        Task::none()
    }

    fn set_query(&mut self, query: String) -> Task<Message> {
        self.query = query;
        let trimmed = self.query.trim().to_string();
        self.global_seq += 1;
        if trimmed.chars().count() < GLOBAL_SEARCH_MIN {
            self.global_hits.clear();
            return self.pump();
        }
        let seq = self.global_seq;
        let ctx = self.ctx.clone();
        let search = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    whatsapp::search_messages(ctx.db().conn(), &trimmed, None, GLOBAL_SEARCH_LIMIT)
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
            },
            move |result| Message::GlobalHits(seq, result),
        );
        Task::batch([search, self.pump()])
    }

    fn refresh(&mut self) -> Task<Message> {
        self.sync_status();
        self.reload_list();
        self.reload_favorites();
        self.refresh_messages();
        let mut tasks = vec![self.pump()];
        if self.history_wait.is_some() {
            tasks.push(self.fetch_older());
        }
        if self.drawer.as_ref().is_some_and(|d| !d.loading) {
            tasks.push(self.drawer_task());
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests;
