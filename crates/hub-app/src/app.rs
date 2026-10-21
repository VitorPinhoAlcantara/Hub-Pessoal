use std::time::Duration;

use hub_core::HubContext;
use hub_core::agenda::Reminder;
use hub_core::chrono::NaiveDateTime;
use hub_core::events::HubEvent;
use hub_core::platform::{TrayEvent, TrayImage};
use hub_core::scheduler;
use hub_ui_kit::{Section, style};
use iced::window::{self, Direction};
use iced::{Element, Size, Subscription, Task, Theme};
use section_agenda::Agenda;

use crate::{Options, app_icon, shell, streams};

const SMOKE_STEP: Duration = Duration::from_millis(1000);
const SMOKE_STEPS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Agenda,
    Scripts,
    Shortcuts,
    WhatsApp,
    Settings,
}

impl Tab {
    pub const SECTIONS: [Tab; 4] = [Tab::Agenda, Tab::Scripts, Tab::Shortcuts, Tab::WhatsApp];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Agenda => "Agenda",
            Tab::Scripts => "Scripts",
            Tab::Shortcuts => "Atalhos",
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
    DragWindow,
    ResizeWindow(Direction),
    Minimize,
    ToggleMaximize,
    MaximizedChanged(bool),
    CloseWindow,
    ToggleAi,
    TogglePlayer,
    Bus(HubEvent),
    SchedulerWake,
    SmokeStep(usize),
}

pub struct App {
    pub ctx: HubContext,
    pub options: Options,
    pub window: Option<window::Id>,
    pub maximized: bool,
    pub tab: Tab,
    pub agenda: Option<Agenda>,
    pub missed: Vec<Reminder>,
    pub next_wake: NaiveDateTime,
    pub ai_open: bool,
    pub player_open: bool,
    pub tray_available: bool,
}

pub fn run(ctx: HubContext, options: Options) -> iced::Result {
    iced::daemon(
        move || App::boot(ctx.clone(), options),
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(App::theme)
    .subscription(App::subscription)
    .settings(iced::Settings {
        default_text_size: hub_ui_kit::tokens::FONT_SIZE.into(),
        default_font: hub_ui_kit::font::UI,
        fonts: hub_ui_kit::font::ALL
            .iter()
            .map(|bytes| (*bytes).into())
            .collect(),
        antialiasing: false,
        ..iced::Settings::default()
    })
    .style(|_, _| iced::theme::Style {
        background_color: hub_ui_kit::tokens::BG_BASE,
        text_color: hub_ui_kit::tokens::TEXT_PRIMARY,
    })
    .run()
}

fn window_settings() -> window::Settings {
    window::Settings {
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
        let tray_available = !options.smoke_test && spawn_tray(&ctx);
        let mut app = Self {
            ctx,
            options,
            window: None,
            maximized: false,
            tab: Tab::Agenda,
            agenda: None,
            missed,
            next_wake,
            ai_open: false,
            player_open: false,
            tray_available,
        };
        let task = if options.start_hidden && app.tray_available {
            Task::none()
        } else {
            app.open_window()
        };
        (app, task)
    }

    fn open_window(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            return window::gain_focus(id);
        }
        let (id, open) = window::open(window_settings());
        self.window = Some(id);
        open.map(Message::WindowOpened)
    }

    fn release_window_state(&mut self) {
        if let Some(agenda) = &mut self.agenda {
            agenda.on_hide();
        }
        self.agenda = None;
        self.maximized = false;
    }

    fn ensure_section(&mut self, tab: Tab) {
        if tab == Tab::Agenda && self.agenda.is_none() {
            self.agenda = Some(Agenda::new(
                self.ctx.clone(),
                std::mem::take(&mut self.missed),
            ));
        }
    }

    fn select_tab(&mut self, tab: Tab) {
        if self.tab != tab
            && self.tab == Tab::Agenda
            && let Some(agenda) = &mut self.agenda
        {
            agenda.on_hide();
        }
        self.tab = tab;
        self.ensure_section(tab);
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
                        return iced::exit();
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
            Message::DragWindow => {
                if let Some(id) = self.window {
                    return window::drag(id);
                }
            }
            Message::ResizeWindow(direction) => {
                if let Some(id) = self.window
                    && !self.maximized
                {
                    return window::drag_resize(id, direction);
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
            Message::ToggleAi => self.ai_open = !self.ai_open,
            Message::TogglePlayer => self.player_open = !self.player_open,
            Message::Bus(event) => return self.on_bus(event),
            Message::SchedulerWake => {
                if let Err(error) = scheduler::tick(&self.ctx) {
                    tracing::warn!(%error, "falha no agendador");
                }
                self.refresh_wake();
                if let Some(agenda) = &mut self.agenda {
                    let _ = agenda.update(section_agenda::Message::Refresh);
                }
            }
            Message::SmokeStep(step) => return self.smoke_step(step),
        }
        Task::none()
    }

    fn on_bus(&mut self, event: HubEvent) -> Task<Message> {
        match event {
            HubEvent::AgendaChanged => {
                self.refresh_wake();
                if let Some(agenda) = &mut self.agenda {
                    return agenda
                        .update(section_agenda::Message::Refresh)
                        .map(Message::Agenda);
                }
            }
            HubEvent::ShowWindow => return self.open_window(),
            HubEvent::QuitRequested => return iced::exit(),
            HubEvent::ConfigChanged => {}
        }
        Task::none()
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
            Tab::Settings,
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
        "hub".to_string()
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
        if self.window.is_some() {
            subscriptions.push(window::close_requests().map(Message::CloseRequested));
            if self.tab == Tab::Agenda
                && let Some(agenda) = &self.agenda
            {
                subscriptions.push(agenda.subscription().map(Message::Agenda));
            }
        }
        Subscription::batch(subscriptions)
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
