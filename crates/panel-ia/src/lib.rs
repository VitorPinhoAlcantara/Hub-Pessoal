pub mod availability;
pub mod codex;
pub mod gemini;
pub mod openai;
pub mod provider;
mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use hub_core::HubContext;
use hub_core::platform::clipboard::{self as clipboard, ClipboardContent};
use hub_ui_kit::iced::task::Handle;
use hub_ui_kit::iced::widget::text_editor::{self, Motion};
use hub_ui_kit::iced::widget::{image, markdown};
use hub_ui_kit::iced::{Element, Subscription, Task};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section, dialog};

pub use availability::Availability;
pub use codex::CodexCli;
pub use gemini::GeminiCli;
pub use hub_core::config::AiProviderKind;
pub use openai::OpenAiCompatible;
pub use provider::{AiEvent, AiProvider, AiRequest, AttachmentInput, ClaudeCli, McpEndpoint};

pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
pub const NO_ANSWER: &str = "A IA terminou sem responder. Confira o provedor em Configurações, aba IA.";
pub const IDLE_ANSWER: &str = "A IA ficou 3 minutos sem dar sinal e a espera foi encerrada. Confira a conexão e o provedor em Configurações, aba IA.";

pub fn with_idle_timeout(
    stream: futures::stream::BoxStream<'static, AiEvent>,
    limit: std::time::Duration,
) -> futures::stream::BoxStream<'static, AiEvent> {
    use futures::StreamExt;
    futures::stream::unfold(Some(stream), move |state| async move {
        let mut stream = state?;
        match tokio::time::timeout(limit, stream.next()).await {
            Ok(Some(event)) => Some((event, Some(stream))),
            Ok(None) => None,
            Err(_) => Some((AiEvent::Error(IDLE_ANSWER.to_string()), None)),
        }
    })
    .boxed()
}

pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_PDF_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub enum AttachmentKind {
    Image {
        media_type: String,
        data: Arc<str>,
        preview: image::Handle,
    },
    Pdf {
        data: Arc<str>,
    },
    Text {
        content: Arc<str>,
    },
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub name: String,
    pub kind: AttachmentKind,
}

impl Attachment {
    pub fn from_bytes(name: String, bytes: Vec<u8>) -> Result<Self, String> {
        let base64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let kind = if let Some(media_type) = media_type_of(&name) {
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(format!("{name} passa de 5 MB"));
            }
            AttachmentKind::Image {
                media_type: media_type.to_string(),
                data: base64(&bytes).into(),
                preview: image::Handle::from_bytes(bytes),
            }
        } else if is_pdf(&name) {
            if bytes.len() > MAX_PDF_BYTES {
                return Err(format!("{name} passa de 20 MB"));
            }
            AttachmentKind::Pdf {
                data: base64(&bytes).into(),
            }
        } else {
            let content = String::from_utf8(bytes)
                .ok()
                .filter(|content| !content.contains('\0'))
                .ok_or_else(|| {
                    format!("{name} não é um tipo suportado (imagem, PDF ou arquivo de texto)")
                })?;
            if content.len() > MAX_TEXT_BYTES {
                return Err(format!("{name} passa de 256 KB de texto"));
            }
            AttachmentKind::Text {
                content: content.into(),
            }
        };
        Ok(Self { name, kind })
    }

    pub fn preview(&self) -> Option<&image::Handle> {
        match &self.kind {
            AttachmentKind::Image { preview, .. } => Some(preview),
            _ => None,
        }
    }

    fn input(&self) -> AttachmentInput {
        match &self.kind {
            AttachmentKind::Image {
                media_type, data, ..
            } => AttachmentInput::Image {
                media_type: media_type.clone(),
                data: data.to_string(),
            },
            AttachmentKind::Pdf { data } => AttachmentInput::Pdf {
                data: data.to_string(),
            },
            AttachmentKind::Text { content } => AttachmentInput::Text {
                name: self.name.clone(),
                content: content.to_string(),
            },
        }
    }
}

pub fn media_type_of(name: &str) -> Option<&'static str> {
    match extension_of(name).as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn is_pdf(name: &str) -> bool {
    extension_of(name) == "pdf"
}

fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

async fn load_attachment(path: PathBuf) -> Result<Attachment, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("falha ao ler {name}: {error}"))?;
    if metadata.is_dir() {
        return Err(format!("{name} é uma pasta; anexe arquivos"));
    }
    if metadata.len() as usize > MAX_PDF_BYTES {
        return Err(format!("{name} é grande demais para anexar"));
    }
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|error| format!("falha ao ler {name}: {error}"))?;
    Attachment::from_bytes(name, bytes)
}

async fn read_clipboard() -> ClipboardContent {
    tokio::task::spawn_blocking(clipboard::read)
        .await
        .unwrap_or_default()
}

pub enum Item {
    User {
        text: String,
        attachments: Vec<Attachment>,
    },
    Assistant {
        raw: String,
        content: markdown::Content,
    },
    Tool {
        id: String,
        name: String,
        input: String,
        output: Option<String>,
        is_error: bool,
    },
    Notice(String),
    Error(String),
}

impl Item {
    fn assistant() -> Self {
        Item::Assistant {
            raw: String::new(),
            content: markdown::Content::new(),
        }
    }

    pub fn copy_text(&self) -> String {
        match self {
            Item::User { text, .. } => text.clone(),
            Item::Assistant { raw, .. } => raw.clone(),
            Item::Tool { output, input, .. } => output.clone().unwrap_or_else(|| input.clone()),
            Item::Notice(text) | Item::Error(text) => text.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    InputEdited(text_editor::Action),
    Send,
    Event(AiEvent),
    Finished,
    Cancel,
    NewConversation,
    LinkClicked(markdown::Uri),
    Copy(usize),
    EditResend(usize),
    ItemMenu(usize),
    MenuClosed,
    AttachFiles,
    PasteAttachments,
    ClipboardRead(ClipboardContent),
    FilesPicked(Vec<PathBuf>),
    FileDropped(PathBuf),
    AttachmentLoaded(Result<Attachment, String>),
    RemoveAttachment(usize),
}

pub struct Chat {
    ctx: HubContext,
    provider: Arc<dyn AiProvider>,
    items: Vec<Item>,
    composer: text_editor::Content,
    attachments: Vec<Attachment>,
    session: Option<String>,
    running: Option<Handle>,
    cancelled: Option<usize>,
    copied: Option<usize>,
    menu: Option<ContextMenu<Message>>,
    width: f32,
}

pub fn short_tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__hub__").unwrap_or(name)
}

impl Chat {
    pub fn new(ctx: HubContext, provider: Arc<dyn AiProvider>) -> Self {
        Self {
            ctx,
            provider,
            items: Vec::new(),
            composer: text_editor::Content::new(),
            attachments: Vec::new(),
            session: None,
            running: None,
            cancelled: None,
            copied: None,
            menu: None,
            width: view::PANEL_WIDTH,
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn set_width(&mut self, width: f32) {
        self.width = width;
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn input(&self) -> String {
        self.composer.text()
    }

    pub fn composer(&self) -> &text_editor::Content {
        &self.composer
    }

    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    pub fn cancelled(&self) -> Option<usize> {
        self.cancelled
    }

    pub fn copied(&self) -> Option<usize> {
        self.copied
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn can_send(&self) -> bool {
        self.running.is_none()
            && (!self.composer.text().trim().is_empty() || !self.attachments.is_empty())
    }

    pub fn provider_label(&self) -> &'static str {
        self.provider.label()
    }

    pub fn set_provider(&mut self, provider: Arc<dyn AiProvider>) {
        let _ = self.update(Message::NewConversation);
        self.provider = provider;
    }

    fn set_input(&mut self, text: &str) {
        self.composer = text_editor::Content::with_text(text);
        self.composer
            .perform(text_editor::Action::Move(Motion::DocumentEnd));
    }

    fn send(&mut self) -> Task<Message> {
        if !self.can_send() {
            return Task::none();
        }
        let prompt = self.composer.text().trim().to_string();
        let attachments = std::mem::take(&mut self.attachments);
        self.composer = text_editor::Content::new();
        self.cancelled = None;
        self.copied = None;
        let inputs = attachments.iter().map(Attachment::input).collect();
        self.items.push(Item::User {
            text: prompt.clone(),
            attachments,
        });
        let stream = with_idle_timeout(
            self.provider.send(AiRequest {
                prompt,
                attachments: inputs,
                session: self.session.clone(),
            }),
            IDLE_TIMEOUT,
        );
        let (task, handle) = Task::run(stream, Message::Event)
            .chain(Task::done(Message::Finished))
            .abortable();
        self.running = Some(handle);
        task
    }

    fn edit_resend(&mut self, index: usize) {
        if self.running.is_some() {
            return;
        }
        let Some(Item::User { text, attachments }) = self.items.get(index) else {
            return;
        };
        let text = text.clone();
        self.attachments = attachments.clone();
        self.set_input(&text);
        self.items.truncate(index);
        self.cancelled = None;
        self.copied = None;
    }

    fn item_menu(&self, index: usize) -> Option<ContextMenu<Message>> {
        let item = self.items.get(index)?;
        let mut items = vec![MenuItem::new("Copiar texto", Message::Copy(index)).icon(Icon::Copy)];
        if matches!(item, Item::User { .. }) {
            let edit =
                MenuItem::new("Editar e reenviar", Message::EditResend(index)).icon(Icon::Edit);
            items.push(if self.running.is_some() {
                edit.disabled()
            } else {
                edit
            });
        }
        Some(ContextMenu::at_cursor(items, Message::MenuClosed))
    }

    fn push_text(&mut self, text: &str) {
        if !matches!(self.items.last(), Some(Item::Assistant { .. })) {
            self.items.push(Item::assistant());
        }
        if let Some(Item::Assistant { raw, content }) = self.items.last_mut() {
            raw.push_str(text);
            content.push_str(text);
        }
    }

    fn apply(&mut self, event: AiEvent) {
        match event {
            AiEvent::Session(id) => self.session = Some(id),
            AiEvent::TextDelta(text) | AiEvent::Text(text) => self.push_text(&text),
            AiEvent::ToolUse { id, name, input } => self.items.push(Item::Tool {
                id,
                name,
                input,
                output: None,
                is_error: false,
            }),
            AiEvent::ToolResult {
                id,
                output,
                is_error,
            } => {
                let target = self.items.iter_mut().rev().find_map(|item| match item {
                    Item::Tool {
                        id: tool_id,
                        output,
                        is_error,
                        ..
                    } if *tool_id == id => Some((output, is_error)),
                    _ => None,
                });
                if let Some((slot, error_flag)) = target {
                    *slot = Some(output);
                    *error_flag = is_error;
                }
            }
            AiEvent::Done {
                session,
                is_error,
                summary,
            } => {
                if let Some(session) = session {
                    self.session = Some(session);
                }
                if is_error {
                    self.items.push(Item::Error(
                        summary.unwrap_or_else(|| "A resposta terminou com erro".to_string()),
                    ));
                }
            }
            AiEvent::Error(message) => self.items.push(Item::Error(message)),
        }
    }
}

impl Section for Chat {
    type Message = Message;

    fn title(&self) -> &str {
        "Chat de IA"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(text) => self.set_input(&text),
            Message::InputEdited(action) => self.composer.perform(action),
            Message::Send => return self.send(),
            Message::Event(event) => self.apply(event),
            Message::Finished => {
                self.running = None;
                if matches!(self.items.last(), Some(Item::User { .. })) {
                    self.items.push(Item::Error(NO_ANSWER.to_string()));
                }
            }
            Message::Cancel => {
                if let Some(handle) = self.running.take() {
                    handle.abort();
                    self.cancelled = self
                        .items
                        .iter()
                        .rposition(|item| matches!(item, Item::User { .. }));
                    self.items
                        .push(Item::Notice("Resposta cancelada".to_string()));
                }
            }
            Message::NewConversation => {
                if let Some(handle) = self.running.take() {
                    handle.abort();
                }
                self.items.clear();
                self.session = None;
                self.cancelled = None;
                self.copied = None;
                self.menu = None;
            }
            Message::LinkClicked(uri) => {
                if let Err(error) = self.ctx.platform.open_url(uri.as_str()) {
                    self.items.push(Item::Error(error.to_string()));
                }
            }
            Message::Copy(index) => {
                self.menu = None;
                if let Some(item) = self.items.get(index) {
                    self.copied = Some(index);
                    return hub_ui_kit::iced::clipboard::write(item.copy_text());
                }
            }
            Message::EditResend(index) => {
                self.menu = None;
                self.edit_resend(index);
            }
            Message::ItemMenu(index) => self.menu = self.item_menu(index),
            Message::MenuClosed => self.menu = None,
            Message::AttachFiles => {
                return Task::perform(dialog::pick_files(), Message::FilesPicked);
            }
            Message::PasteAttachments => {
                return Task::perform(read_clipboard(), Message::ClipboardRead);
            }
            Message::ClipboardRead(content) => {
                if let Some(png) = content.png {
                    return self.update(Message::AttachmentLoaded(Attachment::from_bytes(
                        "imagem colada.png".to_string(),
                        png,
                    )));
                }
                return self.update(Message::FilesPicked(content.files));
            }
            Message::FilesPicked(paths) => {
                return Task::batch(
                    paths.into_iter().map(|path| {
                        Task::perform(load_attachment(path), Message::AttachmentLoaded)
                    }),
                );
            }
            Message::FileDropped(path) => {
                return Task::perform(load_attachment(path), Message::AttachmentLoaded);
            }
            Message::AttachmentLoaded(Ok(attachment)) => self.attachments.push(attachment),
            Message::AttachmentLoaded(Err(error)) => self.items.push(Item::Error(error)),
            Message::RemoveAttachment(index) => {
                if index < self.attachments.len() {
                    self.attachments.remove(index);
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }

    fn on_hide(&mut self) {
        if let Some(handle) = self.running.take() {
            handle.abort();
        }
        self.menu = None;
    }

    fn context_menu(&self) -> Option<ContextMenu<Message>> {
        self.menu.clone()
    }
}

#[cfg(test)]
mod tests;
