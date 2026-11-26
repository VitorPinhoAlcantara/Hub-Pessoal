use std::sync::LazyLock;

use hub_ui_kit::iced::widget::{Column, button, column, container, image, row, space, text};
use hub_ui_kit::iced::{Alignment, Color, Element, Length};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use super::dialogs::{PICKER_WIDTH, emoji_picker};
use crate::model::*;
use crate::{DRAFT_INPUT, Message, WhatsApp};

static EMPTY_DRAFT: LazyLock<Draft> = LazyLock::new(Draft::default);

fn strip<'a>(
    accent_color: Color,
    content: Element<'a, Message>,
    close: Message,
) -> Element<'a, Message> {
    container(
        row![
            container(space())
                .width(3)
                .height(Length::Fill)
                .style(move |_| {
                    container::Style {
                        background: Some(hub_ui_kit::iced::Background::Color(accent_color)),
                        ..container::Style::default()
                    }
                }),
            container(content).width(Length::Fill),
            icon_button(Icon::Close, text_secondary(), Some(close)),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center)
        .height(Length::Shrink),
    )
    .padding([units(1), units(2)])
    .width(Length::Fill)
    .style(style::elevated)
    .into()
}

fn context_strips<'a>(draft: &'a Draft) -> Vec<Element<'a, Message>> {
    let mut strips = Vec::new();
    if draft.edit.is_some() {
        strips.push(strip(
            warning(),
            column![body("Editando mensagem")].into(),
            Message::CancelEdit,
        ));
    }
    if let Some(reply) = &draft.reply {
        strips.push(strip(
            accent(),
            column![
                small(reply.label.as_str()).color(accent()),
                small(reply.preview.as_str()),
            ]
            .spacing(2)
            .into(),
            Message::CancelReply,
        ));
    }
    if let Some(pasted) = &draft.image {
        let preview: Element<'a, Message> = match &pasted.preview {
            Some(handle) => image(handle.clone()).height(64).into(),
            None => icon(Icon::Image, ICON_SIZE + 6.0, text_secondary()),
        };
        strips.push(strip(
            accent(),
            row![preview, small("Imagem colada")]
                .spacing(units(3))
                .align_y(Alignment::Center)
                .into(),
            Message::RemoveImage,
        ));
    }
    if let Some(voice) = &draft.voice {
        strips.push(strip(
            accent(),
            row![
                icon(Icon::Mic, ICON_SIZE, accent()),
                body(format!(
                    "Áudio gravado {}",
                    format_clock(voice.seconds as u64)
                )),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center)
            .into(),
            Message::RemoveVoice,
        ));
    }
    strips
}

fn recording_row(section: &WhatsApp) -> Option<Element<'_, Message>> {
    let elapsed = section.recording_elapsed()?;
    Some(
        row![
            container(space())
                .width(10)
                .height(10)
                .style(style::dot(danger())),
            body(format_clock(elapsed.as_secs())),
            space().width(Length::Fill),
            ghost_button("Cancelar", Some(Message::RecordCancel)),
            primary_icon_button(Icon::Send, "Enviar", Some(Message::RecordStop)),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center)
        .into(),
    )
}

fn suggestions(section: &WhatsApp) -> Option<Element<'_, Message>> {
    let list = section.mention_suggestions();
    if list.is_empty() {
        return None;
    }
    Some(
        container(
            list.into_iter()
                .fold(Column::new().spacing(2), |col, (name, jid)| {
                    let label = if jid.is_none() {
                        format!("@{name} (marca todo mundo)")
                    } else {
                        format!("@{name}")
                    };
                    col.push(
                        button(text(label).size(FONT_SIZE))
                            .padding([units(1), units(2)])
                            .width(Length::Fill)
                            .style(style::ghost)
                            .on_press(Message::MentionPicked(name, jid)),
                    )
                }),
        )
        .padding(units(1))
        .width(Length::Fill)
        .style(style::elevated)
        .into(),
    )
}

pub fn composer(section: &WhatsApp) -> Element<'_, Message> {
    if !section.is_connected() {
        return small("Conecte o WhatsApp para responder.").into();
    }
    let mut stack = Column::new().spacing(units(1));
    let draft = section.current_draft().unwrap_or(&EMPTY_DRAFT);
    for strip in context_strips(draft) {
        stack = stack.push(strip);
    }
    if let Some(list) = suggestions(section) {
        stack = stack.push(list);
    }
    if section.emoji_open && !matches!(section.dialog, Some(Dialog::React(_))) {
        stack = stack.push(
            container(emoji_picker(section))
                .padding(units(2))
                .width(PICKER_WIDTH)
                .style(style::elevated),
        );
    }
    if let Some(recording) = recording_row(section) {
        return stack.push(recording).into();
    }
    let sending = section.sending();
    let placeholder = if draft.image.is_some() {
        "Legenda"
    } else if draft.edit.is_some() {
        "Editar mensagem"
    } else {
        "Mensagem"
    };
    let can_send = draft.can_send() && !sending;
    let action: Element<'_, Message> = if draft.can_send() {
        button(icon(Icon::Send, ICON_SIZE - 2.0, on_accent()))
            .padding(units(2))
            .style(style::primary)
            .on_press_maybe(can_send.then_some(Message::Send))
            .into()
    } else {
        button(icon(Icon::Mic, ICON_SIZE - 2.0, on_accent()))
            .padding(units(2))
            .style(style::primary)
            .on_press_maybe((!sending && draft.edit.is_none()).then_some(Message::RecordStart))
            .into()
    };
    let attach_enabled = !sending && draft.edit.is_none();
    stack
        .push(
            row![
                button(icon(Icon::File, ICON_SIZE - 2.0, text_secondary()))
                    .padding(units(2))
                    .style(style::ghost)
                    .on_press_maybe(attach_enabled.then_some(Message::AttachFiles)),
                button(icon(
                    Icon::Smile,
                    ICON_SIZE - 2.0,
                    if section.emoji_open {
                        accent()
                    } else {
                        text_secondary()
                    }
                ))
                .padding(units(2))
                .style(style::ghost)
                .on_press(Message::EmojiToggle),
                text_field(placeholder, &draft.text, Message::DraftChanged)
                    .id(DRAFT_INPUT)
                    .on_submit(Message::Send),
                action,
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
        )
        .into()
}
