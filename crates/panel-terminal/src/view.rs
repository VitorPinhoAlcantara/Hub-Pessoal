use hub_ui_kit::iced::widget::{Row, button, column, container, row, space, text, tooltip};
use hub_ui_kit::iced::{Alignment, Element, Length};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};
use iced_term::TerminalView;

use crate::{Message, Terminal};

const BAR_HEIGHT: f32 = 30.0;

fn tab<'a>(id: u64, title: &'a str, active: bool) -> Element<'a, Message> {
    let color = if active {
        text_primary()
    } else {
        text_secondary()
    };
    button(
        row![
            icon(Icon::Terminal, ICON_SIZE - 6.0, color),
            text(title).size(FONT_SIZE_SMALL).color(color)
        ]
        .spacing(units(1))
        .align_y(Alignment::Center),
    )
    .padding([units(1), units(2)])
    .style(style::toggle(active))
    .on_press(Message::Select(id))
    .into()
}

fn action<'a>(
    glyph: Icon,
    color: hub_ui_kit::iced::Color,
    label: &'a str,
    message: Message,
) -> Element<'a, Message> {
    with_tooltip(
        button(
            container(icon(glyph, ICON_SIZE - 6.0, color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .width(units(6))
        .height(units(6))
        .padding(0)
        .style(style::ghost)
        .on_press(message),
        label,
        tooltip::Position::Bottom,
    )
}

pub fn view(panel: &Terminal) -> Element<'_, Message> {
    let tabs = panel
        .tab_titles()
        .fold(Row::new().spacing(units(1)), |r, (id, title, active)| {
            r.push(tab(id, title, active))
        });
    let bar = container(
        row![
            tabs,
            space().width(Length::Fill),
            action(
                Icon::Plus,
                text_secondary(),
                "Novo terminal",
                Message::NewSession
            ),
            action(
                Icon::Trash,
                text_secondary(),
                "Encerrar terminal",
                Message::KillActive
            ),
            action(
                Icon::Close,
                text_secondary(),
                "Fechar painel",
                Message::ClosePanel
            ),
        ]
        .spacing(units(1))
        .align_y(Alignment::Center),
    )
    .padding([0.0, units(2)])
    .height(BAR_HEIGHT)
    .width(Length::Fill)
    .center_y(BAR_HEIGHT)
    .style(style::panel);

    let body: Element<'_, Message> = match panel.active_term() {
        Some(term) => TerminalView::show(term).map(Message::Term),
        None => container(muted("Nenhum terminal aberto"))
            .center(Length::Fill)
            .style(style::console)
            .into(),
    };

    column![bar, body]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
