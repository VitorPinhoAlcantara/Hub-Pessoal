use hub_ui_kit::iced::widget::{
    Column, button, column, container, markdown, row, scrollable, space, text,
};
use hub_ui_kit::iced::{Alignment, Element, Font, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::{Chat, Item, Message, PROVIDERS, short_tool_name};

pub const PANEL_WIDTH: f32 = 380.0;

fn markdown_settings() -> markdown::Settings {
    markdown::Settings::with_text_size(
        FONT_SIZE,
        markdown::Style::from_palette(style::theme().palette()),
    )
}

fn bubble<'a>(content: impl Into<Element<'a, Message>>, user: bool) -> Element<'a, Message> {
    let inner = container(content)
        .padding([units(2), units(3)])
        .max_width(PANEL_WIDTH - units(14))
        .style(if user { style::banner } else { style::elevated });
    let row = if user {
        row![space().width(Length::Fill), inner]
    } else {
        row![inner, space().width(Length::Fill)]
    };
    row.into()
}

fn tool_card<'a>(
    name: &'a str,
    input: &'a str,
    output: Option<&'a str>,
    is_error: bool,
) -> Element<'a, Message> {
    let (status, color) = match (output, is_error) {
        (None, _) => ("executando", ACCENT),
        (Some(_), false) => ("ok", SUCCESS),
        (Some(_), true) => ("erro", DANGER),
    };
    let summary: String = output
        .unwrap_or(input)
        .chars()
        .filter(|c| !c.is_control())
        .take(140)
        .collect();
    container(
        column![
            row![
                icon(Icon::Tool, ICON_SIZE - 6.0, color),
                text(short_tool_name(name))
                    .size(FONT_SIZE_SMALL)
                    .font(Font::MONOSPACE)
                    .color(TEXT_PRIMARY),
                space().width(Length::Fill),
                badge(status, color),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
            small(summary),
        ]
        .spacing(units(1)),
    )
    .padding([units(2), units(3)])
    .width(Length::Fill)
    .style(style::outlined(alpha(color, 0.6), false))
    .into()
}

fn item_view(item: &Item) -> Element<'_, Message> {
    match item {
        Item::User(prompt) => bubble(body(prompt.as_str()), true),
        Item::Assistant { content, raw } => {
            if raw.is_empty() {
                bubble(muted("Pensando"), false)
            } else {
                bubble(
                    markdown::view(content.items(), markdown_settings()).map(Message::LinkClicked),
                    false,
                )
            }
        }
        Item::Tool {
            name,
            input,
            output,
            is_error,
            ..
        } => tool_card(name, input, output.as_deref(), *is_error),
        Item::Notice(notice) => container(small(notice.as_str()))
            .center_x(Length::Fill)
            .into(),
        Item::Error(error) => container(
            row![
                icon(Icon::Warning, ICON_SIZE - 4.0, DANGER),
                body(error.as_str()).color(DANGER)
            ]
            .spacing(units(2)),
        )
        .padding([units(2), units(3)])
        .width(Length::Fill)
        .style(style::outlined(DANGER, false))
        .into(),
    }
}

pub fn view(chat: &Chat) -> Element<'_, Message> {
    let providers = PROVIDERS
        .iter()
        .fold(row![].spacing(units(1)), |r, (label, enabled)| {
            let active = *label == chat.provider_label();
            r.push(
                button(text(*label).size(FONT_SIZE_SMALL))
                    .padding([units(1), units(2)])
                    .style(style::toggle(active))
                    .on_press_maybe(
                        enabled
                            .then_some(Message::NewConversation)
                            .filter(|_| !active),
                    ),
            )
        });
    let header = column![
        row![
            icon(Icon::Ai, ICON_SIZE, ACCENT),
            heading("Chat de IA"),
            space().width(Length::Fill),
            icon_label_button(Icon::Plus, "Nova", Some(Message::NewConversation)),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
        providers,
    ]
    .spacing(units(2));

    let messages: Element<'_, Message> = if chat.items().is_empty() {
        container(
            column![
                muted("Peça algo como:"),
                small("me lembra de beber água em 1 minuto"),
                small("o que tenho na agenda amanhã?"),
                small("cria um atalho para o GitHub"),
            ]
            .spacing(units(2)),
        )
        .padding(units(4))
        .center_x(Length::Fill)
        .into()
    } else {
        scrollable(
            container(
                chat.items()
                    .iter()
                    .fold(Column::new().spacing(units(3)), |col, item| {
                        col.push(item_view(item))
                    }),
            )
            .padding(Padding::default().right(units(3))),
        )
        .anchor_bottom()
        .height(Length::Fill)
        .style(style::scroll)
        .into()
    };

    let action: Element<'_, Message> = if chat.is_running() {
        button(icon(Icon::Stop, ICON_SIZE - 2.0, BG_BASE))
            .padding(units(2))
            .style(style::danger_solid)
            .on_press(Message::Cancel)
            .into()
    } else {
        button(icon(Icon::Send, ICON_SIZE - 2.0, BG_BASE))
            .padding(units(2))
            .style(style::primary)
            .on_press_maybe((!chat.input().trim().is_empty()).then_some(Message::Send))
            .into()
    };
    let composer = row![
        text_field("Mensagem", chat.input(), Message::InputChanged).on_submit(Message::Send),
        action,
    ]
    .spacing(units(2))
    .align_y(Alignment::Center);

    container(column![header, container(messages).height(Length::Fill), composer].spacing(units(3)))
        .padding(units(4))
        .width(PANEL_WIDTH)
        .height(Length::Fill)
        .style(style::panel)
        .into()
}
