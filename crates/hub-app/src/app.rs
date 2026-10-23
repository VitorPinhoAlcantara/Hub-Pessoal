use std::time::Duration;

use hub_core::HubContext;
use hub_core::agenda::Reminder;
use hub_core::chrono::NaiveDateTime;
use hub_core::confirm::ConfirmRequest;
use hub_core::events::HubEvent;
use hub_core::platform::{self, MediaCommand, TrayEvent, TrayImage, TrayMenuState, WindowDrag};
use hub_core::player::PlayerCommand;
use hub_core::scheduler;
use hub_core::scripts::Origin;
use hub_core::scripts::runner::{self, RunEvent};
use hub_core::watcher::{self, ConfigWatcher};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section, style};
use iced::keyboard::{self, Modifiers};
use iced::window::{self, Direction};
use iced::{Element, Size, Subscription, Task, Theme};
use iced::{Event, Point, Vector, event, mouse};
use section_agenda::Agenda;
use section_atalhos::Shortcuts;
use section_inicio::{Destination, Home};
use section_notas::Notes;
use section_scripts::Scripts;
use section_whatsapp::WhatsApp;
use std::sync::Arc;

use hub_mcp::McpServer;
use panel_ia::{Chat, ClaudeCli};
use panel_player::Player;

use crate::appearance::{self, Personalization};
use crate::maintenance::{self, Maintenance};
use crate::{Options, app_icon, shell, streams};

const SMOKE_STEP: Duration = Duration::from_millis(1000);
const SMOKE_STEPS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Home,
    Agenda,
    Scripts,
    Shortcuts,
    WhatsApp,
    Notes,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Personalization,
    General,
}

impl Tab {
    pub const SECTIONS: [Tab; 6] = [
        Tab::Home,
        Tab::Agenda,
        Tab::Shortcuts,
        Tab::WhatsApp,
        Tab::Notes,
        Tab::Scripts,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Agenda => "Agenda",
            Tab::Scripts => "Scripts",
            Tab::Home => "Início",
            Tab::Shortcuts => "Atalhos",
            Tab::Notes => "Anotações",
            Tab::WhatsApp => "WhatsApp",
            Tab::Settings => "Configurações",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    CloseRequested(window::Id),
    TabSelected(Tab),
    Agenda(section_agenda::Message),
    Scripts(section_scripts::Message),
    Shortcuts(section_atalhos::Message),
    Notes(section_notas::Message),
    Home(section_inicio::Message),
    WhatsApp(section_whatsapp::Message),
    ScheduledRun(String, RunEvent),
    Chat(panel_ia::Message),
    Player(panel_player::Message),
    ConfirmAnswered(u64, bool),
    Zoom(i32),
    ZoomReset,
    AppearanceMenu,
    ToolsMenu,
    ToggleTerminal,
    Terminal(panel_terminal::Message),
    TerminalResizeStart,
    TerminalResizeMove(Point),
    TerminalResizeEnd,
    MenuClosed,
    ChatResizeStart,
    ChatResizeMove(Point),
    ChatResizeEnd,
    PlayerDragStart,
    PlayerDragMove(Point),
    PlayerDragEnd,
    DragWindow,
    ResizeWindow(Direction),
    NativeDrag(WindowDrag, u64),
    WindowRaw(u64),
    TranslucentConfirmed,
    WallpaperLoaded(Result<Option<appearance::WallpaperFrame>, String>),
    WallpaperBlurred(u64, Option<appearance::WallpaperFrame>),
    WallpaperRefresh,
    Minimize,
    ToggleMaximize,
    MaximizedChanged(bool),
    CloseWindow,
    ToggleAi,
    TogglePlayer,
    Bus(HubEvent),
    SchedulerWake,
    Maintenance(maintenance::Message),
    Appearance(appearance::Message),
    SettingsTab(SettingsTab),
    BrowserLogin(hub_core::config::BrowserLogin),
    OpenPersonalization,
    SmokeStep(usize),
}

pub struct App {
    pub ctx: HubContext,
    pub options: Options,
    pub window: Option<window::Id>,
    pub maximized: bool,
    pub tab: Tab,
    pub agenda: Option<Agenda>,
    pub scripts: Option<Scripts>,
    pub shortcuts: Option<Shortcuts>,
    pub notes: Option<Notes>,
    pub home: Option<Home>,
    pub terminal: Option<panel_terminal::Terminal>,
    pub terminal_open: bool,
    pub terminal_height: f32,
    pub resizing_terminal: bool,
    pub whatsapp: Option<WhatsApp>,
    pub chat: Option<Chat>,
    pub player: Option<Player>,
    pub confirmations: Vec<ConfirmRequest>,
    pub mcp: Result<McpServer, String>,
    pub missed: Vec<Reminder>,
    pub next_wake: NaiveDateTime,
    pub ai_open: bool,
    pub player_open: bool,
    pub tray_available: bool,
    pub watcher: Option<ConfigWatcher>,
    pub zoom: f32,
    pub chat_width: f32,
    pub resizing_chat: bool,
    pub player_offset: Vector,
    pub player_drag: Option<(Point, Vector)>,
    pub toolbar_menu: Option<ContextMenu<Message>>,
    pub maintenance: Maintenance,
    pub personalization: Personalization,
    pub settings_tab: SettingsTab,
    pub wallpaper: Option<appearance::WallpaperFrame>,
    pub wallpaper_generation: u64,
    pub raw_window: Option<u64>,
}

pub fn run(ctx: HubContext, options: Options) -> iced::Result {
    let appearance_config = ctx.config().appearance;
    appearance::start_session(&appearance_config);
    appearance::apply(&appearance_config);
    let ui_font = hub_ui_kit::font::select(&appearance_config.font);
    iced::daemon(
        move || App::boot(ctx.clone(), options),
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .scale_factor(App::scale_factor)
    .settings(iced::Settings {
        default_text_size: hub_ui_kit::tokens::FONT_SIZE.into(),
        default_font: ui_font,
        fonts: hub_ui_kit::font::ALL
            .iter()
            .map(|bytes| (*bytes).into())
            .collect(),
        antialiasing: false,
        ..iced::Settings::default()
    })
    .style(|_, _| iced::theme::Style {
        background_color: if appearance::translucent_session() {
            iced::Color::TRANSPARENT
        } else {
            hub_ui_kit::tokens::bg_base()
        },
        text_color: hub_ui_kit::tokens::text_primary(),
    })
    .run()
}

fn window_settings() -> window::Settings {
    window::Settings {
        transparent: appearance::translucent_session(),
        size: Size::new(1100.0, 720.0),
        min_size: Some(Size::new(760.0, 480.0)),
        decorations: false,
        exit_on_close_request: false,
        icon: window::icon::from_rgba(app_icon::rgba(64), 64, 64).ok(),
        ..window::Settings::default()
    }
}

impl App {
    fn boot(ctx: HubContext, options: Options) -> (Self, Task<Message>) {
        let missed = scheduler::startup(&ctx).unwrap_or_else(|error| {
            tracing::warn!(%error, "falha ao preparar a agenda");
            Vec::new()
        });
        let next_wake = scheduler::next_wake(&ctx).unwrap_or_else(|_| ctx.clock.now());
        hub_core::vault::clear_temp();
        let tray_available = !options.smoke_test && spawn_tray(&ctx);
        if !options.smoke_test && hub_core::whatsapp::service::activated_today(&ctx) {
            ctx.whatsapp.start(&ctx);
        }
        let watcher = match watcher::spawn(ctx.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                tracing::warn!(%error, "não foi possível observar a pasta de configuração");
                None
            }
        };
        let mcp = if options.mcp {
            start_mcp(&ctx, options.smoke_test)
        } else {
            Err("desligado".to_string())
        };
        let appearance = ctx.config().appearance;
        let maintenance = Maintenance::new(&ctx);
        let personalization = Personalization::new(&ctx);
        let maintenance_task = if options.smoke_test {
            Task::none()
        } else {
            Task::batch([
                Maintenance::boot_tasks(&ctx).map(Message::Maintenance),
                wallpaper_task(&ctx, None),
            ])
        };
        let mut app = Self {
            ctx,
            options,
            window: None,
            maximized: false,
            tab: if missed.is_empty() {
                Tab::Home
            } else {
                Tab::Agenda
            },
            agenda: None,
            scripts: None,
            shortcuts: None,
            notes: None,
            home: None,
            terminal: None,
            terminal_open: false,
            terminal_height: appearance.clamped_terminal_height(),
            resizing_terminal: false,
            whatsapp: None,
            chat: None,
            player: None,
            confirmations: Vec::new(),
            mcp,
            missed,
            next_wake,
            ai_open: false,
            player_open: false,
            tray_available,
            watcher,
            zoom: appearance.clamped_zoom(),
            chat_width: appearance.clamped_chat_width(),
            resizing_chat: false,
            player_offset: Vector::ZERO,
            player_drag: None,
            toolbar_menu: None,
            maintenance,
            personalization,
            settings_tab: SettingsTab::Personalization,
            wallpaper: None,
            wallpaper_generation: 0,
            raw_window: None,
        };
        if app.tray_available {
            app.refresh_tray();
        }
        let task = if options.start_hidden && app.tray_available {
            Task::none()
        } else {
            app.open_window()
        };
        (app, Task::batch([task, maintenance_task]))
    }

    fn open_window(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            return window::gain_focus(id);
        }
        let (id, open) = window::open(window_settings());
        self.window = Some(id);
        open.map(Message::WindowOpened)
    }

    fn hide_section(&mut self, tab: Tab) {
        match tab {
            Tab::Agenda => {
                if let Some(section) = &mut self.agenda {
                    section.on_hide();
                }
            }
            Tab::Scripts => {
                if let Some(section) = &mut self.scripts {
                    section.on_hide();
                }
            }
            Tab::Shortcuts => {
                if let Some(section) = &mut self.shortcuts {
                    section.on_hide();
                }
            }
            Tab::Notes => {
                if let Some(section) = &mut self.notes {
                    section.on_hide();
                }
            }
            Tab::Home => {
                if let Some(section) = &mut self.home {
                    section.on_hide();
                }
            }
            Tab::WhatsApp => {
                if let Some(section) = &mut self.whatsapp {
                    section.on_hide();
                }
                self.whatsapp = None;
            }
            Tab::Settings => {}
        }
    }

    fn release_window_state(&mut self) {
        self.hide_section(self.tab);
        self.raw_window = None;
        self.wallpaper = None;
        self.agenda = None;
        self.scripts = None;
        self.shortcuts = None;
        self.notes = None;
        self.home = None;
        self.whatsapp = None;
        if let Some(chat) = &mut self.chat {
            chat.on_hide();
        }
        self.chat = None;
        self.maximized = false;
        self.sync_player_visibility();
    }

    fn ensure_section(&mut self, tab: Tab) {
        match tab {
            Tab::Agenda if self.agenda.is_none() => {
                self.agenda = Some(Agenda::new(
                    self.ctx.clone(),
                    std::mem::take(&mut self.missed),
                ));
            }
            Tab::Scripts if self.scripts.is_none() => {
                self.scripts = Some(Scripts::new(self.ctx.clone()));
            }
            Tab::Shortcuts if self.shortcuts.is_none() => {
                self.shortcuts = Some(Shortcuts::new(self.ctx.clone()));
            }
            Tab::Notes if self.notes.is_none() => {
                self.notes = Some(Notes::new(self.ctx.clone()));
            }
            Tab::Home if self.home.is_none() => {
                self.home = Some(Home::new(self.ctx.clone()));
            }
            Tab::WhatsApp if self.whatsapp.is_none() => {
                if !self.options.smoke_test {
                    self.ctx.whatsapp.start(&self.ctx);
                }
                self.whatsapp = Some(WhatsApp::new(self.ctx.clone()));
            }
            _ => {}
        }
    }

    fn toggle_terminal(&mut self) -> Task<Message> {
        match &mut self.terminal {
            None => {
                let terminal = panel_terminal::Terminal::new(self.ctx.clone());
                let focus = terminal.focus().map(Message::Terminal);
                self.terminal = Some(terminal);
                self.terminal_open = true;
                focus
            }
            Some(terminal) if terminal.session_count() == 0 => {
                self.terminal_open = true;
                terminal
                    .update(panel_terminal::Message::NewSession)
                    .map(Message::Terminal)
            }
            Some(terminal) if !self.terminal_open => {
                self.terminal_open = true;
                terminal.focus().map(Message::Terminal)
            }
            Some(_) => {
                self.terminal_open = false;
                Task::none()
            }
        }
    }

    fn refresh_home(&mut self) {
        if let Some(home) = &mut self.home {
            home.refresh();
        }
    }

    fn navigate(&mut self, destination: Destination) -> Task<Message> {
        match destination {
            Destination::Agenda => self.select_tab(Tab::Agenda),
            Destination::Shortcuts => self.select_tab(Tab::Shortcuts),
            Destination::Notes(note) => {
                self.select_tab(Tab::Notes);
                if let (Some(id), Some(notes)) = (note, &mut self.notes) {
                    return notes
                        .update(section_notas::Message::NoteSelected(id))
                        .map(Message::Notes);
                }
            }
        }
        Task::none()
    }

    fn select_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.hide_section(self.tab);
        }
        self.tab = tab;
        self.ensure_section(tab);
    }

    fn save_appearance(&mut self) {
        let mut config = self.ctx.config();
        config.appearance.zoom = self.zoom;
        config.appearance.chat_width = self.chat_width;
        config.appearance.terminal_height = self.terminal_height;
        if let Err(error) = config.save(&self.ctx.paths.config_file()) {
            tracing::warn!(%error, "falha ao salvar a aparência");
        }
        *self.ctx.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
    }

    pub fn scale_factor(&self, _window: window::Id) -> f32 {
        self.zoom
    }

    fn ensure_player(&mut self) -> Task<Message> {
        if self.player.is_some() {
            return Task::none();
        }
        let (player, task) = Player::new(self.ctx.clone());
        self.player = Some(player);
        task.map(Message::Player)
    }

    fn player_control(&mut self, command: PlayerCommand) -> Task<Message> {
        use panel_player::Message as P;
        let boot = self.ensure_player();
        let Some(player) = &mut self.player else {
            return boot;
        };
        let mpv = player.is_active();
        let paused = player.status().paused;
        let playing = player.system().map(|media| media.playing);
        let system = |command| Some(P::SystemCommand(command));
        let message = match command {
            PlayerCommand::TogglePause if mpv => Some(P::TogglePause),
            PlayerCommand::Pause if mpv => (!paused).then_some(P::TogglePause),
            PlayerCommand::Resume if mpv => paused.then_some(P::TogglePause),
            PlayerCommand::Next if mpv => Some(P::Next),
            PlayerCommand::Previous if mpv => Some(P::Previous),
            PlayerCommand::TogglePause => system(MediaCommand::PlayPause),
            PlayerCommand::Pause if playing == Some(true) => system(MediaCommand::PlayPause),
            PlayerCommand::Resume if playing == Some(false) => system(MediaCommand::PlayPause),
            PlayerCommand::Next => system(MediaCommand::Next),
            PlayerCommand::Previous => system(MediaCommand::Previous),
            PlayerCommand::SetVolume(volume) => Some(P::VolumeChanged(f64::from(volume))),
            PlayerCommand::Stop if mpv => Some(P::Stop),
            PlayerCommand::FavoritesChanged => Some(P::FavoritesReload),
            PlayerCommand::Pause | PlayerCommand::Resume | PlayerCommand::Stop => None,
        };
        let task = match message {
            Some(message) => player.update(message).map(Message::Player),
            None => Task::none(),
        };
        Task::batch([boot, task])
    }

    fn sync_player_visibility(&mut self) {
        let visible = self.player_open && self.window.is_some();
        if let Some(player) = &mut self.player {
            player.set_visible(visible);
        }
    }

    fn ensure_chat(&mut self) {
        if self.chat.is_none() {
            let config = self
                .mcp
                .as_ref()
                .ok()
                .map(|_| hub_mcp::client_config_path(&self.ctx));
            let provider = ClaudeCli::new(self.ctx.paths.data_dir.clone(), config);
            let mut chat = Chat::new(self.ctx.clone(), Arc::new(provider));
            chat.set_width(self.chat_width);
            self.chat = Some(chat);
        }
    }

    fn refresh_wake(&mut self) {
        match scheduler::next_wake(&self.ctx) {
            Ok(wake) => self.next_wake = wake,
            Err(error) => tracing::warn!(%error, "falha ao calcular o próximo evento"),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowOpened(id) => {
                self.ensure_section(self.tab);
                self.sync_player_visibility();
                if !self.options.smoke_test {
                    return Task::batch([
                        window::raw_id::<Message>(id).map(Message::WindowRaw),
                        wallpaper_task(&self.ctx, self.wallpaper.clone()),
                    ]);
                }
                if self.options.smoke_test {
                    tracing::info!(?id, "teste de fumaça: janela aberta");
                    return Task::perform(streams::after(SMOKE_STEP), |_| Message::SmokeStep(1));
                }
            }
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                    self.release_window_state();
                    if !self.tray_available {
                        return self.quit();
                    }
                }
            }
            Message::CloseRequested(id) => return window::close(id),
            Message::CloseWindow => {
                if let Some(id) = self.window {
                    return window::close(id);
                }
            }
            Message::TabSelected(tab) => self.select_tab(tab),
            Message::Agenda(message) => {
                if let Some(agenda) = &mut self.agenda {
                    return agenda.update(message).map(Message::Agenda);
                }
            }
            Message::Scripts(message) => {
                if let Some(scripts) = &mut self.scripts {
                    return scripts.update(message).map(Message::Scripts);
                }
            }
            Message::Home(section_inicio::Message::Navigate(destination)) => {
                return self.navigate(destination);
            }
            Message::Home(message) => {
                if let Some(home) = &mut self.home {
                    return home.update(message).map(Message::Home);
                }
            }
            Message::Notes(message) => {
                if let Some(notes) = &mut self.notes {
                    return notes.update(message).map(Message::Notes);
                }
            }
            Message::Shortcuts(message) => {
                if let Some(shortcuts) = &mut self.shortcuts {
                    return shortcuts.update(message).map(Message::Shortcuts);
                }
            }
            Message::WhatsApp(message) => {
                if let Some(whatsapp) = &mut self.whatsapp {
                    return whatsapp.update(message).map(Message::WhatsApp);
                }
            }
            Message::ScheduledRun(id, event) => {
                if let Some(scripts) = &mut self.scripts {
                    let message = section_scripts::Message::External(id, Origin::Scheduled, event);
                    return scripts.update(message).map(Message::Scripts);
                }
            }
            Message::DragWindow => {
                if let Some(id) = self.window {
                    return window::raw_id::<Message>(id)
                        .map(|raw| Message::NativeDrag(WindowDrag::Move, raw));
                }
            }
            Message::ResizeWindow(direction) => {
                if let Some(id) = self.window
                    && !self.maximized
                {
                    let drag = native_edge(direction);
                    return window::raw_id::<Message>(id)
                        .map(move |raw| Message::NativeDrag(drag, raw));
                }
            }
            Message::NativeDrag(drag, raw) => {
                if platform::begin_window_drag(raw, drag).is_err()
                    && let Some(id) = self.window
                {
                    return match drag {
                        WindowDrag::Move => window::drag(id),
                        _ => window::drag_resize(id, iced_edge(drag)),
                    };
                }
            }
            Message::TranslucentConfirmed => appearance::confirm_translucent_start(&self.ctx.paths),
            Message::WindowRaw(raw) => {
                platform::apply_taskbar_icon(raw);
                self.raw_window = Some(raw);
                if appearance::translucent_session() {
                    self.apply_window_blur();
                    return Task::perform(streams::after(Duration::from_secs(5)), |_| {
                        Message::TranslucentConfirmed
                    });
                }
            }
            Message::Minimize => {
                if let Some(id) = self.window {
                    return window::minimize(id, true);
                }
            }
            Message::ToggleMaximize => {
                if let Some(id) = self.window {
                    return window::toggle_maximize::<Message>(id)
                        .chain(window::is_maximized(id).map(Message::MaximizedChanged));
                }
            }
            Message::MaximizedChanged(maximized) => self.maximized = maximized,
            Message::ToggleAi => {
                self.ai_open = !self.ai_open;
                if self.ai_open {
                    self.ensure_chat();
                }
            }
            Message::Chat(message) => {
                if let Some(chat) = &mut self.chat {
                    return chat.update(message).map(Message::Chat);
                }
            }
            Message::Zoom(steps) => {
                let mut appearance = self.ctx.config().appearance;
                appearance.zoom = self.zoom;
                appearance.step_zoom(steps);
                self.zoom = appearance.clamped_zoom();
                self.toolbar_menu = None;
                self.save_appearance();
            }
            Message::ZoomReset => {
                self.zoom = 1.0;
                self.toolbar_menu = None;
                self.save_appearance();
            }
            Message::AppearanceMenu => {
                let percent = (self.zoom * 100.0).round() as i32;
                let at = Point::new(
                    hub_ui_kit::cursor::position().x,
                    hub_ui_kit::tokens::TOOLBAR_HEIGHT,
                );
                self.toolbar_menu = Some(ContextMenu::new(
                    at,
                    vec![
                        MenuItem::new("Aumentar zoom", Message::Zoom(1))
                            .icon(Icon::Plus)
                            .shortcut("Ctrl +"),
                        MenuItem::new("Diminuir zoom", Message::Zoom(-1))
                            .icon(Icon::Minimize)
                            .shortcut("Ctrl -"),
                        MenuItem::new(format!("Restaurar zoom ({percent}%)"), Message::ZoomReset)
                            .icon(Icon::Refresh)
                            .shortcut("Ctrl 0"),
                        MenuItem::new("Personalização", Message::OpenPersonalization)
                            .icon(Icon::Settings),
                    ],
                    Message::MenuClosed,
                ));
            }
            Message::MenuClosed => self.toolbar_menu = None,
            Message::ToolsMenu => {
                let at = Point::new(
                    hub_ui_kit::cursor::position().x,
                    hub_ui_kit::tokens::TOOLBAR_HEIGHT,
                );
                let label = if self.terminal_open {
                    "Esconder terminal"
                } else {
                    "Abrir terminal"
                };
                self.toolbar_menu = Some(ContextMenu::new(
                    at,
                    vec![
                        MenuItem::new(label, Message::ToggleTerminal)
                            .icon(Icon::Terminal)
                            .shortcut("Ctrl Shift `"),
                    ],
                    Message::MenuClosed,
                ));
            }
            Message::ToggleTerminal => {
                self.toolbar_menu = None;
                return self.toggle_terminal();
            }
            Message::Terminal(panel_terminal::Message::Hide) => self.terminal_open = false,
            Message::Terminal(panel_terminal::Message::ClosePanel) => {
                self.terminal_open = false;
                if let Some(terminal) = &mut self.terminal {
                    return terminal.close_all().map(Message::Terminal);
                }
            }
            Message::Terminal(message) => {
                if let Some(terminal) = &mut self.terminal {
                    let task = terminal.update(message).map(Message::Terminal);
                    if terminal.is_finished() {
                        self.terminal = None;
                    }
                    return task;
                }
            }
            Message::TerminalResizeStart => self.resizing_terminal = true,
            Message::TerminalResizeMove(position) => {
                let viewport = hub_ui_kit::cursor::viewport();
                let height = viewport.height - position.y - hub_ui_kit::tokens::GAP_PANEL;
                let max = (viewport.height - hub_ui_kit::tokens::TOOLBAR_HEIGHT) * 0.75;
                self.terminal_height = height.clamp(
                    hub_core::config::TERMINAL_HEIGHT_MIN,
                    max.max(hub_core::config::TERMINAL_HEIGHT_MIN),
                );
            }
            Message::TerminalResizeEnd => {
                self.resizing_terminal = false;
                self.save_appearance();
            }
            Message::ChatResizeStart => self.resizing_chat = true,
            Message::ChatResizeMove(position) => {
                let viewport = hub_ui_kit::cursor::viewport();
                let width = viewport.width - position.x - hub_ui_kit::tokens::GAP_PANEL;
                self.chat_width = width.clamp(
                    hub_core::config::CHAT_WIDTH_MIN,
                    hub_core::config::CHAT_WIDTH_MAX.min(viewport.width * 0.7),
                );
                if let Some(chat) = &mut self.chat {
                    chat.set_width(self.chat_width);
                }
            }
            Message::ChatResizeEnd => {
                self.resizing_chat = false;
                self.save_appearance();
            }
            Message::PlayerDragStart => {
                self.player_drag = Some((hub_ui_kit::cursor::position(), self.player_offset));
            }
            Message::PlayerDragMove(position) => {
                if let Some((start, base)) = self.player_drag {
                    let viewport = hub_ui_kit::cursor::viewport();
                    self.player_offset = Vector::new(
                        (base.x + start.x - position.x)
                            .clamp(0.0, (viewport.width - 240.0).max(0.0)),
                        (base.y + start.y - position.y)
                            .clamp(0.0, (viewport.height - 160.0).max(0.0)),
                    );
                }
            }
            Message::PlayerDragEnd => self.player_drag = None,
            Message::ConfirmAnswered(id, approved) => {
                self.ctx.confirmations.answer(id, approved);
                self.confirmations.retain(|c| c.id != id);
            }
            Message::TogglePlayer => {
                self.player_open = !self.player_open;
                let task = self.ensure_player();
                self.sync_player_visibility();
                return task;
            }
            Message::Player(message) => {
                if let Some(player) = &mut self.player {
                    return player.update(message).map(Message::Player);
                }
            }
            Message::Bus(event) => return self.on_bus(event),
            Message::SchedulerWake => {
                let due = match scheduler::tick(&self.ctx) {
                    Ok(tick) => tick.scripts,
                    Err(error) => {
                        tracing::warn!(%error, "falha no agendador");
                        Vec::new()
                    }
                };
                self.refresh_wake();
                if let Some(agenda) = &mut self.agenda {
                    let _ = agenda.update(section_agenda::Message::Refresh);
                }
                let runs: Vec<Task<Message>> = due
                    .into_iter()
                    .map(|script| {
                        let id = script.id.clone();
                        let (events, _cancel) =
                            runner::run(self.ctx.clone(), script, Origin::Scheduled);
                        Task::run(events, move |event| {
                            Message::ScheduledRun(id.clone(), event)
                        })
                    })
                    .collect();
                let daily = Maintenance::daily(&self.ctx).map(Message::Maintenance);
                return Task::batch(runs.into_iter().chain([daily]));
            }
            Message::Maintenance(message) => {
                return match self.maintenance.update(&self.ctx, message) {
                    maintenance::Outcome::None => Task::none(),
                    maintenance::Outcome::Task(task) => task.map(Message::Maintenance),
                    maintenance::Outcome::Quit => self.quit(),
                };
            }
            Message::Appearance(message) => {
                let outcome = self.personalization.update(&self.ctx, message);
                if let Some(terminal) = &mut self.terminal {
                    terminal.apply_theme();
                }
                return match outcome {
                    appearance::Outcome::None => Task::none(),
                    appearance::Outcome::Restart => {
                        Task::done(Message::Maintenance(maintenance::Message::Restart))
                    }
                    appearance::Outcome::Wallpaper => {
                        self.wallpaper_generation += 1;
                        wallpaper_task(&self.ctx, self.wallpaper.clone())
                    }
                    appearance::Outcome::WindowBlur => {
                        self.apply_window_blur();
                        Task::none()
                    }
                    appearance::Outcome::BlurPreview(blur) => match self.wallpaper.clone() {
                        Some(current) => {
                            self.wallpaper_generation += 1;
                            let generation = self.wallpaper_generation;
                            Task::perform(appearance::blur_wallpaper(current, blur), move |frame| {
                                Message::WallpaperBlurred(generation, frame)
                            })
                        }
                        None => Task::none(),
                    },
                    appearance::Outcome::Task(task) => task.map(Message::Appearance),
                };
            }
            Message::WallpaperRefresh => {
                return wallpaper_task(&self.ctx, self.wallpaper.clone());
            }
            Message::WallpaperLoaded(Ok(None)) => self.wallpaper = None,
            Message::WallpaperLoaded(Ok(Some(frame))) => {
                let same = self
                    .wallpaper
                    .as_ref()
                    .is_some_and(|current| current.key == frame.key && current.blur == frame.blur);
                if !same {
                    self.wallpaper = Some(frame);
                }
            }
            Message::WallpaperBlurred(generation, frame) => {
                if generation == self.wallpaper_generation
                    && let Some(frame) = frame
                {
                    self.wallpaper = Some(frame);
                }
            }
            Message::WallpaperLoaded(Err(error)) => {
                tracing::warn!(%error, "falha ao preparar o papel de parede");
                self.wallpaper = None;
            }
            Message::SettingsTab(tab) => self.settings_tab = tab,
            Message::BrowserLogin(browser) => {
                let mut config = self.ctx.config();
                config.player.browser_login = browser;
                if let Err(error) = config.save(&self.ctx.paths.config_file()) {
                    tracing::warn!(%error, "falha ao salvar o navegador do player");
                }
                *self.ctx.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
            }
            Message::OpenPersonalization => {
                self.toolbar_menu = None;
                self.tab = Tab::Settings;
                self.settings_tab = SettingsTab::Personalization;
            }
            Message::SmokeStep(step) => return self.smoke_step(step),
        }
        Task::none()
    }

    fn on_bus(&mut self, event: HubEvent) -> Task<Message> {
        match event {
            HubEvent::NotesChanged => {
                if let Some(notes) = &mut self.notes {
                    notes.refresh();
                }
                self.refresh_home();
            }
            HubEvent::AgendaChanged => {
                self.refresh_wake();
                self.refresh_home();
                if let Some(agenda) = &mut self.agenda {
                    return agenda
                        .update(section_agenda::Message::Refresh)
                        .map(Message::Agenda);
                }
            }
            HubEvent::ScriptsChanged => {
                self.refresh_wake();
                self.refresh_tray();
                if let Some(scripts) = &mut self.scripts {
                    return scripts
                        .update(section_scripts::Message::Refresh)
                        .map(Message::Scripts);
                }
            }
            HubEvent::ShortcutsChanged => {
                self.refresh_home();
                if let Some(shortcuts) = &mut self.shortcuts {
                    return shortcuts
                        .update(section_atalhos::Message::Refresh)
                        .map(Message::Shortcuts);
                }
            }
            HubEvent::ConfirmationRequested(request) => {
                self.confirmations.push(request);
                return self.open_window();
            }
            HubEvent::ConfirmationClosed(id) => self.confirmations.retain(|c| c.id != id),
            HubEvent::PlayRequested(url) => {
                self.player_open = true;
                let boot = self.ensure_player();
                self.sync_player_visibility();
                let play = match &mut self.player {
                    Some(player) => player
                        .update(panel_player::Message::Play(url))
                        .map(Message::Player),
                    None => Task::none(),
                };
                return Task::batch([boot, play]);
            }
            HubEvent::PlayerControl(command) => return self.player_control(command),
            HubEvent::WhatsAppChanged => {
                if let Some(whatsapp) = &mut self.whatsapp {
                    return whatsapp
                        .update(section_whatsapp::Message::Refresh)
                        .map(Message::WhatsApp);
                }
            }
            HubEvent::MuteRequested(hours) => self.ctx.mute_notifications(hours),
            HubEvent::UnmuteRequested => self.ctx.unmute_notifications(),
            HubEvent::NotificationsChanged => self.refresh_tray(),
            HubEvent::RunScriptRequested(id) => match self.ctx.script(&id) {
                Ok(script) => {
                    let (events, _cancel) = runner::run(self.ctx.clone(), script, Origin::Manual);
                    return Task::run(events, move |event| {
                        Message::ScheduledRun(id.clone(), event)
                    });
                }
                Err(error) => tracing::warn!(%error, "falha ao rodar o script pela bandeja"),
            },
            HubEvent::ShowWindow => return self.open_window(),
            HubEvent::QuitRequested => return self.quit(),
            HubEvent::WhatsAppOpen(jid) => {
                self.hide_section(self.tab);
                self.tab = Tab::WhatsApp;
                self.ensure_section(Tab::WhatsApp);
                if let Some(whatsapp) = &mut self.whatsapp {
                    whatsapp.open_conversation(jid);
                }
                return self.open_window();
            }
            HubEvent::ConfigChanged => {
                appearance::apply(&self.ctx.config().appearance);
                if let Some(terminal) = &mut self.terminal {
                    terminal.apply_theme();
                }
                return wallpaper_task(&self.ctx, self.wallpaper.clone());
            }
        }
        Task::none()
    }

    fn apply_window_blur(&self) {
        let Some(raw) = self.raw_window.filter(|_| appearance::translucent_session()) else {
            return;
        };
        let blur = self.ctx.config().appearance.wallpaper.window_blur;
        if let Err(error) = platform::apply_window_blur(raw, blur, appearance::acrylic_tint()) {
            tracing::warn!(%error, "falha ao aplicar o desfoque da janela");
        }
    }

    fn quit(&mut self) -> Task<Message> {
        self.ctx.whatsapp.shutdown();
        hub_core::vault::clear_temp();
        iced::exit()
    }

    fn refresh_tray(&self) {
        let scripts = self
            .ctx
            .scripts()
            .into_iter()
            .map(|script| (script.id, script.meta.name))
            .collect();
        let muted_until = self
            .ctx
            .notifications_muted_until()
            .map(|until| until.format("%H:%M").to_string());
        self.ctx.platform.update_tray(TrayMenuState {
            scripts,
            muted_until,
        });
    }

    fn smoke_step(&mut self, step: usize) -> Task<Message> {
        if step >= SMOKE_STEPS {
            tracing::info!("teste de fumaça concluído");
            return iced::exit();
        }
        let tabs = [
            Tab::Scripts,
            Tab::Shortcuts,
            Tab::WhatsApp,
            Tab::Notes,
            Tab::Settings,
            Tab::Home,
            Tab::Agenda,
        ];
        self.select_tab(tabs[(step - 1) % tabs.len()]);
        self.ai_open = step % 2 == 1;
        self.player_open = step.is_multiple_of(2);
        Task::perform(streams::after(SMOKE_STEP), move |_| {
            Message::SmokeStep(step + 1)
        })
    }

    pub fn title(&self, _window: window::Id) -> String {
        hub_core::channel::app_name().to_string()
    }

    pub fn theme(&self, _window: window::Id) -> Theme {
        style::theme()
    }

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        shell::view(self)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(self.ctx.events.clone(), streams::bus_events).map(Message::Bus),
            Subscription::run_with(self.next_wake, streams::wake_at)
                .map(|_| Message::SchedulerWake),
            window::close_events().map(Message::WindowClosed),
        ];
        if let Some(player) = &self.player {
            subscriptions.push(player.subscription().map(Message::Player));
        }
        if let Some(terminal) = &self.terminal {
            subscriptions.push(terminal.subscription().map(Message::Terminal));
        }
        if self.window.is_some() {
            subscriptions.push(window::close_requests().map(Message::CloseRequested));
            if self.ctx.config().appearance.wallpaper.mode
                == hub_core::config::WallpaperMode::System
            {
                subscriptions.push(
                    iced::time::every(Duration::from_secs(60)).map(|_| Message::WallpaperRefresh),
                );
            }
            subscriptions.push(event::listen_with(shortcut_keys));
            if self.resizing_chat {
                subscriptions.push(event::listen_with(resize_events));
            }
            if self.resizing_terminal {
                subscriptions.push(event::listen_with(terminal_resize_events));
            }
            if self.player_drag.is_some() {
                subscriptions.push(event::listen_with(player_drag_events));
            }
            if self.ai_open {
                subscriptions.push(event::listen_with(file_drops));
            }
            let section = match self.tab {
                Tab::Agenda => self
                    .agenda
                    .as_ref()
                    .map(|s| s.subscription().map(Message::Agenda)),
                Tab::Scripts => self
                    .scripts
                    .as_ref()
                    .map(|s| s.subscription().map(Message::Scripts)),
                Tab::Shortcuts => self
                    .shortcuts
                    .as_ref()
                    .map(|s| s.subscription().map(Message::Shortcuts)),
                Tab::Notes => self
                    .notes
                    .as_ref()
                    .map(|s| s.subscription().map(Message::Notes)),
                Tab::Home => self
                    .home
                    .as_ref()
                    .map(|s| s.subscription().map(Message::Home)),
                Tab::WhatsApp => self
                    .whatsapp
                    .as_ref()
                    .map(|s| s.subscription().map(Message::WhatsApp)),
                Tab::Settings => None,
            };
            subscriptions.extend(section);
        }
        Subscription::batch(subscriptions)
    }
}

fn shortcut_keys(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed {
        key,
        modifiers,
        physical_key,
        ..
    }) = event
    else {
        return None;
    };
    if !modifiers.contains(Modifiers::CTRL) {
        return None;
    }
    let backquote = physical_key == keyboard::key::Physical::Code(keyboard::key::Code::Backquote)
        || matches!(key.as_ref(), keyboard::Key::Character("`" | "~"));
    if modifiers.contains(Modifiers::SHIFT) && backquote {
        return Some(Message::ToggleTerminal);
    }
    match key.as_ref() {
        keyboard::Key::Character("+" | "=") => Some(Message::Zoom(1)),
        keyboard::Key::Character("-" | "_") => Some(Message::Zoom(-1)),
        keyboard::Key::Character("0") => Some(Message::ZoomReset),
        _ => None,
    }
}

fn terminal_resize_events(
    event: Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::TerminalResizeMove(position))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        | Event::Mouse(mouse::Event::CursorLeft) => Some(Message::TerminalResizeEnd),
        _ => None,
    }
}

fn file_drops(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Window(window::Event::FileDropped(path)) => {
            Some(Message::Chat(panel_ia::Message::FileDropped(path)))
        }
        _ => None,
    }
}

fn resize_events(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::ChatResizeMove(position))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        | Event::Mouse(mouse::Event::CursorLeft) => Some(Message::ChatResizeEnd),
        _ => None,
    }
}

fn player_drag_events(
    event: Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::PlayerDragMove(position))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        | Event::Mouse(mouse::Event::CursorLeft) => Some(Message::PlayerDragEnd),
        _ => None,
    }
}

fn start_mcp(ctx: &HubContext, ephemeral: bool) -> Result<McpServer, String> {
    let token = hub_mcp::ensure_token(ctx).map_err(|e| e.to_string())?;
    let port = if ephemeral { 0 } else { ctx.config().mcp.port };
    let server = hub_mcp::start(ctx.clone(), port, token).map_err(|e| e.to_string())?;
    if let Err(error) = hub_mcp::write_client_config(ctx, &server) {
        tracing::warn!(%error, "falha ao gravar mcp.json");
    }
    Ok(server)
}

fn wallpaper_task(
    ctx: &HubContext,
    current: Option<appearance::WallpaperFrame>,
) -> Task<Message> {
    Task::perform(
        appearance::load_wallpaper(ctx.clone(), current),
        Message::WallpaperLoaded,
    )
}

fn native_edge(direction: Direction) -> WindowDrag {
    match direction {
        Direction::North => WindowDrag::North,
        Direction::South => WindowDrag::South,
        Direction::East => WindowDrag::East,
        Direction::West => WindowDrag::West,
        Direction::NorthEast => WindowDrag::NorthEast,
        Direction::NorthWest => WindowDrag::NorthWest,
        Direction::SouthEast => WindowDrag::SouthEast,
        Direction::SouthWest => WindowDrag::SouthWest,
    }
}

fn iced_edge(drag: WindowDrag) -> Direction {
    match drag {
        WindowDrag::North | WindowDrag::Move => Direction::North,
        WindowDrag::South => Direction::South,
        WindowDrag::East => Direction::East,
        WindowDrag::West => Direction::West,
        WindowDrag::NorthEast => Direction::NorthEast,
        WindowDrag::NorthWest => Direction::NorthWest,
        WindowDrag::SouthEast => Direction::SouthEast,
        WindowDrag::SouthWest => Direction::SouthWest,
    }
}

fn spawn_tray(ctx: &HubContext) -> bool {
    let bus = ctx.events.clone();
    let image = TrayImage {
        rgba: app_icon::rgba(32),
        width: 32,
        height: 32,
    };
    let result = ctx.platform.spawn_tray(
        image,
        Box::new(move |event| {
            bus.publish(match event {
                TrayEvent::Open => HubEvent::ShowWindow,
                TrayEvent::Quit => HubEvent::QuitRequested,
                TrayEvent::Mute(hours) => HubEvent::MuteRequested(hours),
                TrayEvent::Unmute => HubEvent::UnmuteRequested,
                TrayEvent::RunScript(id) => HubEvent::RunScriptRequested(id),
            })
        }),
    );
    match result {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "bandeja indisponível, fechar a janela encerra o hub");
            false
        }
    }
}

#[cfg(test)]
mod tests;
