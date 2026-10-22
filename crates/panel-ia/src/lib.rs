pub mod provider;
mod view;

use std::sync::Arc;

use hub_core::HubContext;
use hub_ui_kit::Section;
use hub_ui_kit::iced::task::Handle;
use hub_ui_kit::iced::widget::markdown;
use hub_ui_kit::iced::{Element, Subscription, Task};

pub use provider::{AiEvent, AiProvider, AiRequest, ClaudeCli};

pub const PROVIDERS: [(&str, bool); 3] = [
    ("Claude CLI", true),
    ("Codex", false),
    ("API ou modelo local", false),
];

pub enum Item {
    User(String),
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
}

#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    Send,
    Event(AiEvent),
    Finished,
    Cancel,
    NewConversation,
    LinkClicked(markdown::Uri),
}

pub struct Chat {
    ctx: HubContext,
    provider: Arc<dyn AiProvider>,
    items: Vec<Item>,
    input: String,
    session: Option<String>,
    running: Option<Handle>,
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
            input: String::new(),
            session: None,
            running: None,
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn provider_label(&self) -> &'static str {
        self.provider.label()
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
            Message::InputChanged(text) => self.input = text,
            Message::Send => {
                let prompt = self.input.trim().to_string();
                if prompt.is_empty() || self.running.is_some() {
                    return Task::none();
                }
                self.input.clear();
                self.items.push(Item::User(prompt.clone()));
                let stream = self.provider.send(AiRequest {
                    prompt,
                    session: self.session.clone(),
                });
                let (task, handle) = Task::run(stream, Message::Event)
                    .chain(Task::done(Message::Finished))
                    .abortable();
                self.running = Some(handle);
                return task;
            }
            Message::Event(event) => self.apply(event),
            Message::Finished => self.running = None,
            Message::Cancel => {
                if let Some(handle) = self.running.take() {
                    handle.abort();
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
            }
            Message::LinkClicked(uri) => {
                if let Err(error) = self.ctx.platform.open_url(uri.as_str()) {
                    self.items.push(Item::Error(error.to_string()));
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
    }
}

#[cfg(test)]
mod tests;
