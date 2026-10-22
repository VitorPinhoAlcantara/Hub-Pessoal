use hub_core::config::Renderer;
use hub_core::confirm::ConfirmRequest;
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{ContextMenu, Icon, Section, cursor, menu, style};
use iced::widget::{
    Column, button, column, container, mouse_area, opaque, row, space, stack, text,
};
use iced::window::Direction;
use iced::{Alignment, Element, Length, Padding, mouse};

use crate::app::{App, Message, Tab};

const EDGE: f32 = 5.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let body = row![sidebar(app), main_area(app)].height(Length::Fill);
    let window = container(column![toolbar(app), body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::base);
    let mut layers = stack![window];
    if let Some(menu) = active_menu(app) {
        layers = layers.push(menu::view(&menu));
    }
    if let Some(request) = app.confirmations.first() {
        layers = layers.push(confirmation(request, app.confirmations.len()));
    }
    if !app.maximized {
        layers = layers.push(resize_edges());
    }
    cursor::tracker(layers).into()
}

fn active_menu(app: &App) -> Option<ContextMenu<Message>> {
    if let Some(menu) = &app.toolbar_menu {
        return Some(menu.clone());
    }
    if app.ai_open
        && let Some(menu) = app.chat.as_ref().and_then(|c| c.context_menu())
    {
        return Some(menu.map(Message::Chat));
    }
    match app.tab {
        Tab::Agenda => app
            .agenda
            .as_ref()
            .and_then(|s| s.context_menu())
            .map(|m| m.map(Message::Agenda)),
        Tab::Scripts => app
            .scripts
            .as_ref()
            .and_then(|s| s.context_menu())
            .map(|m| m.map(Message::Scripts)),
        Tab::Shortcuts => app
            .shortcuts
            .as_ref()
            .and_then(|s| s.context_menu())
            .map(|m| m.map(Message::Shortcuts)),
        Tab::WhatsApp => app
            .whatsapp
            .as_ref()
            .and_then(|s| s.context_menu())
            .map(|m| m.map(Message::WhatsApp)),
        Tab::Settings => None,
    }
}

fn confirmation(request: &ConfirmRequest, pending: usize) -> Element<'_, Message> {
    let mut content = column![
        row![
            icon(Icon::Ai, ICON_SIZE, ACCENT),
            heading(request.title.as_str())
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
        muted(request.detail.as_str()),
        small("Sem resposta em 60 segundos o pedido é recusado."),
    ]
    .spacing(units(3));
    if pending > 1 {
        content = content.push(small(format!("Mais {} pedidos na fila", pending - 1)));
    }
    content = content.push(
        row![
            space().width(Length::Fill),
            ghost_button("Recusar", Some(Message::ConfirmAnswered(request.id, false))),
            primary_button("Permitir", Some(Message::ConfirmAnswered(request.id, true))),
        ]
        .spacing(units(2)),
    );
    let dialog = container(content.width(400))
        .padding(units(5))
        .style(style::dialog);
    opaque(
        container(opaque(dialog))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(style::backdrop),
    )
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
    let menu_button = |label: &'static str, message: Message| {
        button(text(label).size(FONT_SIZE).color(TEXT_SECONDARY))
            .padding([units(1), units(2) + 2.0])
            .style(style::ghost)
            .on_press(message)
    };
    let bar = row![
        space().width(units(4)),
        body("hub").size(FONT_SIZE_HEADING),
        space().width(units(6)),
        menu_button("Aparência", Message::AppearanceMenu),
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
        Tab::Scripts => match &app.scripts {
            Some(scripts) => scripts.view().map(Message::Scripts),
            None => panel(muted("Carregando")).into(),
        },
        Tab::Shortcuts => match &app.shortcuts {
            Some(shortcuts) => shortcuts.view().map(Message::Shortcuts),
            None => panel(muted("Carregando")).into(),
        },
        Tab::WhatsApp => match &app.whatsapp {
            Some(whatsapp) => whatsapp.view().map(Message::WhatsApp),
            None => panel(muted("Carregando")).into(),
        },
        Tab::Settings => settings(app),
    };
    let mut panels = row![content].height(Length::Fill);
    if app.ai_open
        && let Some(chat) = &app.chat
    {
        let handle =
            mouse_area(
                container(container(space()).width(2).height(units(10)).style(
                    if app.resizing_chat {
                        style::indicator
                    } else {
                        style::grip
                    },
                ))
                .width(GAP_PANEL)
                .height(Length::Fill)
                .center_y(Length::Fill)
                .center_x(GAP_PANEL),
            )
            .on_press(Message::ChatResizeStart)
            .interaction(mouse::Interaction::ResizingHorizontally);
        panels = panels.push(row![handle, chat.view().map(Message::Chat)]);
    }
    let area = container(panels)
        .padding(Padding::default().right(GAP_PANEL).bottom(GAP_PANEL))
        .width(Length::Fill)
        .height(Length::Fill);
    match (&app.player, app.player_open) {
        (Some(player), true) => stack![area, player_overlay(app, player)].into(),
        _ => area.into(),
    }
}

fn player_overlay<'a>(app: &'a App, player: &'a panel_player::Player) -> Element<'a, Message> {
    let card = mouse_area(player.view().map(Message::Player))
        .on_press(Message::PlayerDragStart)
        .interaction(if app.player_drag.is_some() {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Grab
        });
    container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_right(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(
            Padding::default()
                .right(GAP_PANEL * 3.0 + app.player_offset.x)
                .bottom(GAP_PANEL * 3.0 + app.player_offset.y),
        )
        .into()
}

fn status(active: bool) -> String {
    if active { "Ativa" } else { "Indisponível" }.to_string()
}

fn settings(app: &App) -> Element<'_, Message> {
    let config = app.ctx.config();
    let renderer = match config.general.renderer {
        Renderer::Gpu => "GPU (wgpu)",
        Renderer::Software => "Software (tiny-skia)",
    };
    let line = |label: &'static str, value: String| {
        row![muted(label).width(200), body(value)].spacing(units(3))
    };
    let mcp = match &app.mcp {
        Ok(server) => server.url(),
        Err(error) => format!("Desligado: {error}"),
    };
    panel(
        column![
            title("Configurações"),
            muted("As opções ficam em config.toml e são recarregadas sozinhas quando o arquivo muda. O renderizador vale na próxima abertura."),
            space().height(units(2)),
            line("Renderizador", renderer.to_string()),
            line(
                "Iniciar com o sistema",
                if config.general.start_with_system { "Sim" } else { "Não" }.to_string()
            ),
            line("Servidor MCP", mcp),
            line("Configuração", app.ctx.paths.config_file().display().to_string()),
            line("Dados", app.ctx.paths.data_dir.display().to_string()),
            line("Bandeja", status(app.tray_available)),
            line("Recarga automática", status(app.watcher.is_some())),
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
