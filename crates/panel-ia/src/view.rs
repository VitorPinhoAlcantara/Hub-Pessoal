use hub_ui_kit::iced::keyboard::{Key, key};
use hub_ui_kit::iced::widget::text_editor::{Binding, KeyPress};
use hub_ui_kit::iced::widget::{
    Column, Row, button, column, container, image, markdown, mouse_area, row, scrollable, space,
    text, text_editor,
};
use hub_ui_kit::iced::{Alignment, Element, Font, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::{Attachment, Chat, Item, Message, PROVIDERS, short_tool_name};

pub const PANEL_WIDTH: f32 = 380.0;
const PREVIEW_SIZE: f32 = 64.0;

fn markdown_settings() -> markdown::Settings {
    markdown::Settings::with_text_size(
        FONT_SIZE,
        markdown::Style::from_palette(style::theme().palette()),
    )
}

fn bubble<'a>(content: impl Into<Element<'a, Message>>, user: bool) -> Element<'a, Message> {
    let inner = container(content)
        .padding([units(2), units(3)])
        .max_width(f32::MAX)
        .style(if user { style::banner } else { style::elevated });
    let row = if user {
        row![space().width(Length::Fill), inner]
    } else {
        row![inner, space().width(Length::Fill)]
    };
    row.into()
}

fn previews<'a>(attachments: &'a [Attachment], removable: bool) -> Element<'a, Message> {
    attachments
        .iter()
        .enumerate()
        .fold(Row::new().spacing(units(2)), |r, (index, attachment)| {
            let thumb: Element<'a, Message> = match attachment.preview() {
                Some(preview) => image(preview.clone())
                    .width(PREVIEW_SIZE)
                    .height(PREVIEW_SIZE)
                    .content_fit(hub_ui_kit::iced::ContentFit::Cover)
                    .into(),
                None => container(
                    column![
                        icon(Icon::File, ICON_SIZE, TEXT_SECONDARY),
                        text(attachment.name.as_str())
                            .size(FONT_SIZE_SMALL)
                            .color(TEXT_SECONDARY)
                            .wrapping(text::Wrapping::None)
                    ]
                    .spacing(units(1))
                    .align_x(Alignment::Center),
                )
                .padding(units(1))
                .width(PREVIEW_SIZE)
                .height(PREVIEW_SIZE)
                .center_y(PREVIEW_SIZE)
                .clip(true)
                .style(style::elevated)
                .into(),
            };
            let cell: Element<'a, Message> = if removable {
                column![
                    thumb,
                    button(text("remover").size(FONT_SIZE_SMALL).color(TEXT_SECONDARY))
                        .padding(0)
                        .style(style::ghost)
                        .on_press(Message::RemoveAttachment(index)),
                ]
                .spacing(units(1))
                .align_x(Alignment::Center)
                .into()
            } else {
                thumb
            };
            r.push(cell)
        })
        .wrap()
        .into()
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

fn small_action<'a>(glyph: Icon, label: &'a str, message: Message) -> Element<'a, Message> {
    button(
        row![
            icon(glyph, ICON_SIZE - 8.0, TEXT_SECONDARY),
            text(label).size(FONT_SIZE_SMALL).color(TEXT_SECONDARY),
        ]
        .spacing(units(1))
        .align_y(Alignment::Center),
    )
    .padding([0.0, units(1)])
    .style(style::ghost)
    .on_press(message)
    .into()
}

fn item_view<'a>(chat: &'a Chat, index: usize, item: &'a Item) -> Element<'a, Message> {
    let copied = chat.copied() == Some(index);
    let copy_label = if copied { "Copiado" } else { "Copiar" };
    let content: Element<'a, Message> = match item {
        Item::User { text, attachments } => {
            let mut inner = Column::new().spacing(units(2));
            if !attachments.is_empty() {
                inner = inner.push(previews(attachments, false));
            }
            if !text.is_empty() {
                inner = inner.push(body(text.as_str()));
            }
            let mut col = column![bubble(inner, true)].spacing(units(1));
            if chat.cancelled() == Some(index) && !chat.is_running() {
                col = col.push(row![
                    space().width(Length::Fill),
                    small_action(Icon::Edit, "Editar e reenviar", Message::EditResend(index)),
                ]);
            }
            col.into()
        }
        Item::Assistant { content, raw } => {
            if raw.is_empty() {
                bubble(muted("Pensando"), false)
            } else {
                let answer = bubble(
                    markdown::view(content.items(), markdown_settings()).map(Message::LinkClicked),
                    false,
                );
                let streaming = chat.is_running() && index + 1 == chat.items().len();
                if streaming {
                    answer
                } else {
                    column![
                        answer,
                        row![small_action(Icon::Copy, copy_label, Message::Copy(index))],
                    ]
                    .spacing(units(1))
                    .into()
                }
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
    };
    mouse_area(content)
        .on_right_press(Message::ItemMenu(index))
        .into()
}

fn composer_binding(press: KeyPress) -> Option<Binding<Message>> {
    if matches!(press.status, text_editor::Status::Focused { .. })
        && matches!(press.key, Key::Named(key::Named::Enter))
        && !press.modifiers.shift()
    {
        return Some(Binding::Custom(Message::Send));
    }
    if matches!(press.status, text_editor::Status::Focused { .. })
        && press.modifiers.command()
        && matches!(&press.key, Key::Character(c) if c.as_str() == "v")
    {
        return Some(Binding::Sequence(vec![
            Binding::Paste,
            Binding::Custom(Message::PasteAttachments),
        ]));
    }
    Binding::from_key_press(press)
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
                space().height(units(2)),
                small("Enter envia, Shift+Enter quebra a linha."),
                small("Cole com Ctrl+V, arraste arquivos para cá ou use o clipe para anexar."),
                small("Clique com o botão direito numa mensagem para copiar ou reenviar."),
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
                    .enumerate()
                    .fold(Column::new().spacing(units(3)), |col, (index, item)| {
                        col.push(item_view(chat, index, item))
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
            .on_press_maybe(chat.can_send().then_some(Message::Send))
            .into()
    };
    let editor = text_editor(chat.composer())
        .placeholder("Mensagem (Shift+Enter quebra a linha)")
        .on_action(Message::InputEdited)
        .key_binding(composer_binding)
        .padding([units(2), units(3)])
        .size(FONT_SIZE)
        .min_height(units(9))
        .max_height(units(40))
        .style(style::editor);
    let attach = icon_button(Icon::Attach, TEXT_SECONDARY, Some(Message::AttachFiles));
    let mut composer = Column::new().spacing(units(2));
    if !chat.attachments().is_empty() {
        composer = composer.push(previews(chat.attachments(), true));
    }
    composer = composer.push(
        row![attach, editor, action]
            .spacing(units(2))
            .align_y(Alignment::End),
    );

    container(column![header, container(messages).height(Length::Fill), composer].spacing(units(3)))
        .padding(units(4))
        .width(chat.width())
        .height(Length::Fill)
        .style(style::panel)
        .into()
}
