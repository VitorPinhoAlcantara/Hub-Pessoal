use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hub_core::chrono::{NaiveDateTime, TimeDelta};
use hub_core::whatsapp::{Message as ChatMessage, MessageKind, Reaction};
use hub_ui_kit::iced::widget::image::Handle;

pub const EDIT_WINDOW_MINUTES: i64 = 15;
pub const AUTO_MEDIA_LIMIT: u64 = 10 * 1024 * 1024;
pub const AUTO_MEDIA_RECENT: usize = 12;
pub const MAX_PARALLEL_DOWNLOADS: usize = 2;
pub const MAX_PARALLEL_PHOTOS: usize = 3;
pub const MAX_PREVIEWS: usize = 80;
pub const PREVIEW_SIZE: u32 = 640;
pub const AVATAR_SIZE: u32 = 96;
pub const STICKER_PICKER: usize = 30;
pub const STICKER_PREVIEW: u32 = 128;
pub const PHOTO_CANDIDATES: usize = 30;
pub const WINDOW_SIZE: usize = 60;
pub const WINDOW_STEP: usize = 60;
pub const JUMP_TAIL: usize = 3;
pub const OLDER_BATCH: usize = 100;
pub const JUMP_BATCH: usize = 300;
pub const JUMP_LIMIT: usize = 3000;
pub const HISTORY_TIMEOUT: Duration = Duration::from_secs(30);
pub const SEARCH_LIMIT: usize = 100;
pub const GLOBAL_SEARCH_LIMIT: usize = 30;
pub const GLOBAL_SEARCH_MIN: usize = 2;
pub const DRAWER_LOAD: usize = 1500;
pub const SPEEDS: [f32; 3] = [1.0, 1.5, 2.0];
pub const QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];
pub const CHAT_SCROLL: &str = "whatsapp-conversa";

const DANGEROUS_EXTENSIONS: &[&str] = &[
    "exe",
    "msi",
    "bat",
    "cmd",
    "ps1",
    "vbs",
    "js",
    "lnk",
    "scr",
    "com",
    "jar",
    "hta",
    "reg",
    "sh",
    "desktop",
    "appimage",
    "pif",
    "vbe",
    "jse",
    "wsf",
    "wsh",
    "cpl",
    "dll",
    "msp",
    "psm1",
    "appx",
    "msix",
    "url",
    "inf",
    "scf",
    "application",
    "gadget",
    "bash",
    "command",
    "apk",
];

pub fn is_dangerous(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let file = file.trim_end_matches(['.', ' ']);
    match file.rsplit_once('.') {
        Some((_, extension)) => {
            let extension = extension.to_ascii_lowercase();
            DANGEROUS_EXTENSIONS.contains(&extension.as_str())
        }
        None => false,
    }
}

pub fn can_edit(message: &ChatMessage, now: NaiveDateTime) -> bool {
    message.mine
        && !message.deleted
        && message.kind == MessageKind::Text
        && message.downloadable
        && now - message.at <= TimeDelta::minutes(EDIT_WINDOW_MINUTES)
}

pub fn visible_range(len: usize, end: Option<usize>, shown: usize) -> Range<usize> {
    let end = end.unwrap_or(len).min(len);
    end.saturating_sub(shown)..end
}

pub fn merge_messages(
    current: Vec<ChatMessage>,
    fresh: Vec<ChatMessage>,
) -> (Vec<ChatMessage>, bool) {
    let Some(first) = fresh.first() else {
        return (fresh, false);
    };
    match current.iter().position(|m| m.id == first.id) {
        Some(index) => {
            let mut merged = current;
            merged.truncate(index);
            merged.extend(fresh);
            (merged, true)
        }
        None => (fresh, false),
    }
}

pub fn group_reactions(list: &[Reaction]) -> Vec<(String, usize, bool)> {
    let mut groups: Vec<(String, usize, bool)> = Vec::new();
    for reaction in list {
        match groups
            .iter_mut()
            .find(|(emoji, _, _)| *emoji == reaction.emoji)
        {
            Some(group) => {
                group.1 += 1;
                group.2 |= reaction.mine;
            }
            None => groups.push((reaction.emoji.clone(), 1, reaction.mine)),
        }
    }
    groups
}

pub fn toggle_destination(selected: &mut Vec<String>, jid: &str) {
    match selected.iter().position(|j| j == jid) {
        Some(index) => {
            selected.remove(index);
        }
        None => selected.push(jid.to_string()),
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn day_label(date: hub_core::chrono::NaiveDate, today: hub_core::chrono::NaiveDate) -> String {
    match (today - date).num_days() {
        0 => "Hoje".to_string(),
        1 => "Ontem".to_string(),
        _ => date.format("%d/%m/%Y").to_string(),
    }
}

#[derive(Clone)]
pub struct Pasted {
    pub png: Arc<Vec<u8>>,
    pub preview: Option<Handle>,
}

impl std::fmt::Debug for Pasted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pasted({} bytes)", self.png.len())
    }
}

#[derive(Debug, Clone)]
pub struct Voice {
    pub path: PathBuf,
    pub seconds: u32,
    pub waveform: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyRef {
    pub id: String,
    pub label: String,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditRef {
    pub id: String,
    pub stash: String,
}

#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub text: String,
    pub reply: Option<ReplyRef>,
    pub edit: Option<EditRef>,
    pub image: Option<Pasted>,
    pub voice: Option<Voice>,
    pub mentions: Vec<(String, String)>,
}

impl Draft {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.reply.is_none()
            && self.edit.is_none()
            && self.image.is_none()
            && self.voice.is_none()
    }

    pub fn can_send(&self) -> bool {
        self.voice.is_some() || self.image.is_some() || !self.text.trim().is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Outgoing {
    pub jid: String,
    pub text: String,
    pub reply: Option<ReplyRef>,
    pub edit: Option<EditRef>,
    pub image: Option<Pasted>,
    pub voice: Option<Voice>,
    pub files: Vec<PathBuf>,
    pub mentions: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct MediaFile {
    pub path: PathBuf,
    pub preview: Option<Handle>,
}

#[derive(Debug, Clone)]
pub enum Media {
    Loading,
    Ready(Option<Handle>, PathBuf),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Show,
    Open,
    Play,
}

#[derive(Debug, Clone)]
pub enum Photo {
    Loading,
    Ready(Handle),
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    React(String),
    Forward(ForwardState),
    ConfirmRevoke(String),
    ConfirmDeleteLocal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardState {
    pub id: String,
    pub origin: String,
    pub query: String,
    pub selected: Vec<String>,
    pub sending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawerTab {
    Media,
    Documents,
    Audios,
}

impl DrawerTab {
    pub fn accepts(self, message: &ChatMessage) -> bool {
        if message.deleted {
            return false;
        }
        match self {
            DrawerTab::Media => matches!(message.kind, MessageKind::Image | MessageKind::Video),
            DrawerTab::Documents => message.kind == MessageKind::Document,
            DrawerTab::Audios => message.kind == MessageKind::Audio,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Drawer {
    pub tab: DrawerTab,
    pub items: Vec<ChatMessage>,
    pub loading: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub results: Vec<ChatMessage>,
    pub index: usize,
    pub seq: u64,
    pub loading: bool,
}

#[derive(Debug, Clone)]
pub struct HistoryWait {
    pub jid: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Default)]
pub struct EmojiState {
    pub query: String,
    pub category: usize,
    pub stickers: bool,
    pub section: hub_core::whatsapp::StickerSection,
}

pub fn trim_previews(order: &mut Vec<String>, keep: &HashSet<String>, limit: usize) -> Vec<String> {
    let mut dropped = Vec::new();
    while order.len() > limit {
        let position = order.iter().position(|id| !keep.contains(id)).unwrap_or(0);
        dropped.push(order.remove(position));
    }
    dropped
}
