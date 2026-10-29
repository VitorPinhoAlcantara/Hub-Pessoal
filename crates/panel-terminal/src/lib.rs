use hub_core::HubContext;
use hub_core::platform;
use hub_ui_kit::iced::{Font, Subscription, Task};
use hub_ui_kit::tokens::*;
use iced_term::settings::{BackendSettings, FontSettings, Settings, ThemeSettings};
use iced_term::{ColorPalette, Command, TerminalView};

mod tabs;
mod view;

pub use tabs::{Removal, Tabs};

const FONT_SIZE_TERMINAL: f32 = 13.0;
const REAP_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Debug, Clone)]
pub enum Message {
    Term(iced_term::Event),
    NewSession,
    Select(u64),
    KillActive,
    ClosePanel,
    Reap,
}

struct Session {
    id: u64,
    title: String,
    term: iced_term::Terminal,
}

pub struct Terminal {
    _ctx: HubContext,
    sessions: Vec<Session>,
    retired: Vec<Session>,
    tabs: Tabs,
    next_id: u64,
    counter: usize,
}

fn hex_string(color: hub_ui_kit::iced::Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn ansi_colors(dark: bool) -> [&'static str; 16] {
    if dark {
        [
            "#3b3f46", "#e06c75", "#8fbf6a", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#c8ccd4",
            "#6b717d", "#f07f89", "#a5d683", "#f0d08c", "#7ac0ff", "#d78ff0", "#6fcdd8", "#ffffff",
        ]
    } else {
        [
            "#2b2f36", "#c0392b", "#2e7d32", "#9a6700", "#1f5fbf", "#8e3aa8", "#0e7c86", "#8a8f98",
            "#555b66", "#d6453a", "#3d9a41", "#b07a00", "#2f78d8", "#a64ec1", "#1a98a4", "#c4c8cf",
        ]
    }
}

fn palette() -> ColorPalette {
    let background = console_bg();
    let dark = luminance(background) < 0.5;
    let c = ansi_colors(dark);
    let foreground = hex_string(text_primary());
    let s = |value: &str| value.to_string();
    ColorPalette {
        foreground: foreground.clone(),
        background: hex_string(background),
        black: s(c[0]),
        red: s(c[1]),
        green: s(c[2]),
        yellow: s(c[3]),
        blue: s(c[4]),
        magenta: s(c[5]),
        cyan: s(c[6]),
        white: s(c[7]),
        bright_black: s(c[8]),
        bright_red: s(c[9]),
        bright_green: s(c[10]),
        bright_yellow: s(c[11]),
        bright_blue: s(c[12]),
        bright_magenta: s(c[13]),
        bright_cyan: s(c[14]),
        bright_white: s(c[15]),
        bright_foreground: None,
        dim_foreground: hex_string(text_secondary()),
        dim_black: s(c[0]),
        dim_red: s(c[1]),
        dim_green: s(c[2]),
        dim_yellow: s(c[3]),
        dim_blue: s(c[4]),
        dim_magenta: s(c[5]),
        dim_cyan: s(c[6]),
        dim_white: s(c[7]),
    }
}

fn terminal_font() -> Font {
    match platform::terminal_font_family() {
        Some(family) => Font::with_name(family),
        None => Font::MONOSPACE,
    }
}

fn settings() -> Settings {
    let (program, args) = platform::default_shell();
    Settings {
        font: FontSettings {
            size: FONT_SIZE_TERMINAL,
            font_type: terminal_font(),
            ..FontSettings::default()
        },
        theme: ThemeSettings::new(Box::new(palette())),
        backend: BackendSettings {
            program,
            args,
            working_directory: platform::home_dir(),
            ..BackendSettings::default()
        },
    }
}

impl Terminal {
    pub fn new(ctx: HubContext) -> Self {
        let mut panel = Self {
            _ctx: ctx,
            sessions: Vec::new(),
            retired: Vec::new(),
            tabs: Tabs::default(),
            next_id: 1,
            counter: 0,
        };
        panel.open_session();
        panel
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_finished(&self) -> bool {
        self.sessions.is_empty() && self.retired.is_empty()
    }

    pub fn focus(&self) -> Task<Message> {
        match self.active_session() {
            Some(session) => TerminalView::focus(session.term.widget_id().clone()),
            None => Task::none(),
        }
    }

    pub fn apply_theme(&mut self) {
        for session in &mut self.sessions {
            session
                .term
                .handle(Command::ChangeTheme(Box::new(palette())));
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Term(iced_term::Event::BackendCall(id, command)) => {
                let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) else {
                    return Task::none();
                };
                match session.term.handle(Command::ProxyToBackend(command)) {
                    iced_term::actions::Action::Shutdown => self.close_session(id),
                    _ => Task::none(),
                }
            }
            Message::NewSession => {
                if self.open_session() {
                    self.focus()
                } else {
                    Task::none()
                }
            }
            Message::Select(id) => {
                self.tabs.select(id);
                self.focus()
            }
            Message::KillActive => match self.tabs.active() {
                Some(id) => self.close_session(id),
                None => Task::done(Message::ClosePanel),
            },
            Message::Reap => {
                self.retired.clear();
                Task::none()
            }
            Message::ClosePanel => Task::none(),
        }
    }

    pub fn view(&self) -> hub_ui_kit::iced::Element<'_, Message> {
        view::view(self)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(
            self.sessions
                .iter()
                .map(|session| session.term.subscription().map(Message::Term)),
        )
    }

    fn active_session(&self) -> Option<&Session> {
        let id = self.tabs.active()?;
        self.sessions.iter().find(|session| session.id == id)
    }

    fn open_session(&mut self) -> bool {
        let id = self.next_id;
        match iced_term::Terminal::new(id, settings()) {
            Ok(term) => {
                self.next_id += 1;
                self.counter += 1;
                self.sessions.push(Session {
                    id,
                    title: format!("Terminal {}", self.counter),
                    term,
                });
                self.tabs.add(id);
                true
            }
            Err(error) => {
                tracing::warn!("falha ao abrir o terminal: {error}");
                false
            }
        }
    }

    fn close_session(&mut self, id: u64) -> Task<Message> {
        if let Some(index) = self.sessions.iter().position(|session| session.id == id) {
            self.retired.push(self.sessions.remove(index));
        }
        let next = match self.tabs.remove(id) {
            Removal::Empty => Task::done(Message::ClosePanel),
            Removal::Activated(_) => self.focus(),
            Removal::Unchanged => Task::none(),
        };
        Task::batch([next, reap_later()])
    }

    pub(crate) fn tab_titles(&self) -> impl Iterator<Item = (u64, &str, bool)> {
        let active = self.tabs.active();
        self.tabs.ids().iter().filter_map(move |id| {
            self.sessions
                .iter()
                .find(|session| session.id == *id)
                .map(|session| {
                    (
                        session.id,
                        session.title.as_str(),
                        active == Some(session.id),
                    )
                })
        })
    }

    pub(crate) fn active_term(&self) -> Option<&iced_term::Terminal> {
        self.active_session().map(|session| &session.term)
    }
}

fn reap_later() -> Task<Message> {
    Task::perform(tokio::time::sleep(REAP_DELAY), |_| Message::Reap)
}

#[cfg(test)]
mod tests;
