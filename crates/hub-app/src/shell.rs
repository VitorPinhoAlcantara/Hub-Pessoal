use hub_core::config::Renderer;
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, Section, style};
use iced::widget::{Column, button, column, container, mouse_area, opaque, row, space, stack};
use iced::window::Direction;
use iced::{Alignment, Element, Length, Padding, mouse};

use crate::app::{App, Message, Tab};

const EDGE: f32 = 5.0;
const AI_PANEL_WIDTH: f32 = 340.0;
const PLAYER_WIDTH: f32 = 300.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let body = row![sidebar(app), main_area(app)].height(Length::Fill);
    let window = container(column![toolbar(app), body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::base);
    if app.maximized {
        window.into()
    } else {
        stack![window, resize_edges()].into()
    }
}

fn toolbar(app: &App) -> Element<'_, Message> {
    let toggle = |glyph: Icon, label: &'static str, active: bool, message: Message| {
        let color = if active { ACCENT } else { TEXT_SECONDARY };
        with_tooltip(
            icon_button(glyph, color, Some(message)).style(style::toggle(active)),
            label,
            iced::widget::tooltip::Position::Bottom,
        )
    };
    let window_button = |glyph: Icon, message: Message| {
        button(
            container(icon(glyph, ICON_SIZE - 4.0, TEXT_SECONDARY))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(46)
        .height(Length::Fill)
        .padding(0)
        .on_press(message)
    };
    let maximize_icon = if app.maximized {
        Icon::Restore
    } else {
        Icon::Maximize
    };
    let bar = row![
        space().width(units(4)),
        body("hub").size(FONT_SIZE_HEADING),
        space().width(Length::Fill),
        row![
            toggle(
                Icon::Player,
                "Música",
                app.player_open,
                Message::TogglePlayer
            ),
            toggle(Icon::Ai, "Chat de IA", app.ai_open, Message::ToggleAi),
        ]
        .spacing(units(1))
        .align_y(Alignment::Center),
        space().width(units(3)),
        window_button(Icon::Minimize, Message::Minimize).style(style::ghost),
        window_button(maximize_icon, Message::ToggleMaximize).style(style::ghost),
        window_button(Icon::Close, Message::CloseWindow).style(style::window_close),
    ]
    .align_y(Alignment::Center)
    .height(TOOLBAR_HEIGHT);
    mouse_area(bar)
        .on_press(Message::DragWindow)
        .on_double_click(Message::ToggleMaximize)
        .into()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let item = |tab: Tab| {
        let glyph = match tab {
            Tab::Agenda => Icon::Agenda,
            Tab::Scripts => Icon::Scripts,
            Tab::Shortcuts => Icon::Shortcuts,
            Tab::WhatsApp => Icon::WhatsApp,
            Tab::Settings => Icon::Settings,
        };
        sidebar_button(
            glyph,
            tab.label(),
            app.tab == tab,
            Message::TabSelected(tab),
        )
    };
    let tabs = Tab::SECTIONS
        .into_iter()
        .fold(Column::new().spacing(units(1)), |col, tab| {
            col.push(item(tab))
        });
    container(
        column![tabs, space().height(Length::Fill), item(Tab::Settings)]
            .padding(Padding::default().bottom(GAP_PANEL)),
    )
    .width(SIDEBAR_WIDTH)
    .height(Length::Fill)
    .into()
}

fn main_area(app: &App) -> Element<'_, Message> {
    let content: Element<'_, Message> = match app.tab {
        Tab::Agenda => match &app.agenda {
            Some(agenda) => agenda.view().map(Message::Agenda),
            None => panel(muted("Carregando")).into(),
        },
        Tab::Scripts => placeholder("Scripts", "A aba de scripts chega na Fase 2."),
        Tab::Shortcuts => placeholder("Atalhos", "A aba de atalhos chega na Fase 2."),
        Tab::WhatsApp => placeholder("WhatsApp", "A aba do WhatsApp chega na Fase 5."),
        Tab::Settings => settings(app),
    };
    let mut panels = row![content].spacing(GAP_PANEL).height(Length::Fill);
    if app.ai_open {
        panels = panels.push(
            container(
                column![
                    heading("Chat de IA"),
                    muted("O chat com o Claude CLI chega na Fase 3."),
                ]
                .spacing(units(2)),
            )
            .padding(units(4))
            .width(AI_PANEL_WIDTH)
            .height(Length::Fill)
            .style(style::panel),
        );
    }
    let area = container(panels)
        .padding(Padding::default().right(GAP_PANEL).bottom(GAP_PANEL))
        .width(Length::Fill)
        .height(Length::Fill);
    if app.player_open {
        stack![area, player_overlay()].into()
    } else {
        area.into()
    }
}

fn player_overlay<'a>() -> Element<'a, Message> {
    let card = container(
        row![
            icon(Icon::Player, ICON_SIZE, ACCENT),
            column![body("Player"), small("Chega na Fase 4")].spacing(2),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding(units(3))
    .width(PLAYER_WIDTH)
    .style(style::elevated);
    container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(
            Padding::default()
                .right(GAP_PANEL * 3.0)
                .bottom(GAP_PANEL * 3.0),
        )
        .into()
}

fn placeholder<'a>(name: &'a str, detail: &'a str) -> Element<'a, Message> {
    panel(column![title(name), muted(detail)].spacing(units(2))).into()
}

fn settings(app: &App) -> Element<'_, Message> {
    let config = app.ctx.config();
    let renderer = match config.general.renderer {
        Renderer::Gpu => "GPU (wgpu)",
        Renderer::Software => "Software (tiny-skia)",
    };
    let line = |label: &'static str, value: String| {
        row![muted(label).width(200), body(value),].spacing(units(3))
    };
    panel(
        column![
            title("Configurações"),
            muted("Por enquanto as opções são editadas em config.toml. A tela de edição chega nas próximas fases."),
            space().height(units(2)),
            line("Renderizador", renderer.to_string()),
            line(
                "Iniciar com o sistema",
                if config.general.start_with_system { "Sim" } else { "Não" }.to_string()
            ),
            line("Porta do MCP", config.mcp.port.to_string()),
            line("Configuração", app.ctx.paths.config_file().display().to_string()),
            line("Dados", app.ctx.paths.data_dir.display().to_string()),
            line(
                "Bandeja",
                if app.tray_available { "Ativa" } else { "Indisponível" }.to_string()
            ),
        ]
        .spacing(units(2)),
    )
    .into()
}

fn resize_edges<'a>() -> Element<'a, Message> {
    let edge = |direction: Direction, width: Length, height: Length| {
        let interaction = match direction {
            Direction::North | Direction::South => mouse::Interaction::ResizingVertically,
            Direction::East | Direction::West => mouse::Interaction::ResizingHorizontally,
            Direction::NorthWest | Direction::SouthEast => {
                mouse::Interaction::ResizingDiagonallyDown
            }
            Direction::NorthEast | Direction::SouthWest => mouse::Interaction::ResizingDiagonallyUp,
        };
        mouse_area(container(space()).width(width).height(height))
            .on_press(Message::ResizeWindow(direction))
            .interaction(interaction)
    };
    let fill = Length::Fill;
    let thin = Length::Fixed(EDGE);
    let corner = Length::Fixed(EDGE * 2.0);
    column![
        row![
            edge(Direction::NorthWest, corner, thin),
            edge(Direction::North, fill, thin),
            edge(Direction::NorthEast, corner, thin),
        ],
        row![
            edge(Direction::West, thin, fill),
            space().width(fill),
            edge(Direction::East, thin, fill),
        ]
        .height(fill),
        row![
            edge(Direction::SouthWest, corner, thin),
            edge(Direction::South, fill, thin),
            edge(Direction::SouthEast, corner, thin),
        ],
    ]
    .into()
}
