use hub_core::whatsapp::{Message as ChatMessage, MessageKind};
use hub_ui_kit::iced::widget::{
    Column, button, column, container, image, mouse_area, row, slider, space, stack, text,
};
use hub_ui_kit::iced::{Alignment, Background, Border, Color, Element, Length, mouse};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use super::WHATSAPP_GREEN;
use crate::media::can_fetch;
use crate::model::*;
use crate::{Message, WhatsApp};

const IMAGE_WIDTH: f32 = 320.0;
const STICKER_WIDTH: f32 = 140.0;
const VIDEO_WIDTH: f32 = 280.0;
const PLACEHOLDER_HEIGHT: f32 = 120.0;
const INLINE_CHARS: usize = 40;
const WIDE_CHARS: usize = 75;
const AUDIO_SLIDER: f32 = 150.0;
const SPEED_LABELS: [&str; 3] = ["1x", "1,5x", "2x"];

fn placeholder<'a>(label: &'a str, width: f32) -> Element<'a, Message> {
    container(small(label))
        .width(width)
        .height(PLACEHOLDER_HEIGHT)
        .center_x(width)
        .center_y(PLACEHOLDER_HEIGHT)
        .style(style::elevated)
        .into()
}

fn failure<'a>(error: &'a str, retry: Message) -> Element<'a, Message> {
    column![
        small(error).color(danger()),
        icon_label_button(Icon::Refresh, "Tentar de novo", Some(retry)),
    ]
    .spacing(units(1))
    .into()
}

fn size_label(message: &ChatMessage) -> String {
    if message.size == 0 {
        String::new()
    } else {
        format!(" ({})", format_size(message.size))
    }
}

fn image_view<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let id = message.id.clone();
    match section.media(&message.id) {
        Some(Media::Ready(Some(handle), _)) => mouse_area(image(handle.clone()).width(IMAGE_WIDTH))
            .on_press(Message::MediaOpen(id))
            .interaction(mouse::Interaction::Pointer)
            .into(),
        Some(Media::Ready(None, _)) => {
            icon_label_button(Icon::Image, "Abrir imagem", Some(Message::MediaOpen(id))).into()
        }
        Some(Media::Loading) => placeholder("Baixando imagem", IMAGE_WIDTH),
        Some(Media::Failed(error)) => failure(error, Message::MediaRequested(id)),
        None => button(
            row![
                icon(Icon::Download, ICON_SIZE - 4.0, text_secondary()),
                text(format!("Baixar imagem{}", size_label(message))).size(FONT_SIZE),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
        )
        .padding([units(2), units(3)])
        .style(style::ghost)
        .on_press(Message::MediaRequested(id))
        .into(),
    }
}

fn sticker_view<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    match section.media(&message.id) {
        Some(Media::Ready(Some(handle), _)) => image(handle.clone()).width(STICKER_WIDTH).into(),
        Some(Media::Loading) => small("Carregando figurinha").into(),
        Some(Media::Failed(_)) | Some(Media::Ready(None, _)) => small("[figurinha]").into(),
        None => icon_label_button(
            Icon::Image,
            "Ver figurinha",
            Some(Message::MediaRequested(message.id.clone())),
        )
        .into(),
    }
}

fn video_view<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let id = message.id.clone();
    let loading = matches!(section.media(&message.id), Some(Media::Loading));
    let duration = if message.duration > 0 {
        format_clock(message.duration as u64)
    } else {
        String::new()
    };
    let detail = format!("{duration}{}", size_label(message));
    if let Some(Some(handle)) = section.video_thumbs.get(&message.id) {
        let badge = container(icon(Icon::Play, ICON_SIZE + 6.0, Color::WHITE))
            .padding(units(3))
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.55))),
                border: Border {
                    radius: 999.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            });
        let thumb = stack![
            image(handle.clone()).width(VIDEO_WIDTH),
            container(badge)
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        ];
        let mut preview = mouse_area(thumb).interaction(mouse::Interaction::Pointer);
        if !loading {
            preview = preview.on_press(Message::MediaOpen(id.clone()));
        }
        let label = if loading {
            "Baixando vídeo".to_string()
        } else {
            detail
        };
        let mut block = column![preview, small(label)].spacing(units(1));
        if let Some(Media::Failed(error)) = section.media(&message.id) {
            block = block.push(failure(error, Message::MediaOpen(id)));
        }
        return block.into();
    }
    let card = button(
        row![
            container(icon(Icon::Video, ICON_SIZE + 6.0, accent())).padding(units(2)),
            column![
                body(if loading { "Baixando vídeo" } else { "Vídeo" }),
                small(detail),
            ]
            .spacing(2),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding(units(2))
    .style(style::ghost)
    .on_press_maybe((!loading).then(|| Message::MediaOpen(id.clone())));
    match section.media(&message.id) {
        Some(Media::Failed(error)) => column![card, failure(error, Message::MediaOpen(id))]
            .spacing(units(1))
            .into(),
        _ => card.into(),
    }
}

fn audio_view<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let id = message.id.clone();
    let progress = section.progress_of(&message.id, message.duration);
    let playing = progress.as_ref().is_some_and(|p| !p.paused);
    let loading = matches!(section.media(&message.id), Some(Media::Loading));
    let glyph = if playing { Icon::Pause } else { Icon::Play };
    let toggle = button(icon(glyph, ICON_SIZE - 2.0, on_accent()))
        .padding(units(2))
        .style(style::primary)
        .on_press_maybe((!loading).then(|| Message::AudioToggle(id.clone())));
    let (fraction, label) = match &progress {
        Some(p) => (
            p.fraction,
            format!(
                "{} / {}",
                format_clock(p.position.as_secs()),
                format_clock(p.total.as_secs())
            ),
        ),
        None => (0.0, format_clock(message.duration as u64)),
    };
    let seek_id = id.clone();
    let seek = slider(0.0..=1.0, fraction, move |value| {
        Message::AudioSeeking(seek_id.clone(), value)
    })
    .step(0.001_f32)
    .on_release(Message::AudioSeekCommit)
    .width(AUDIO_SLIDER)
    .style(style::seek);
    let mut line = row![toggle, seek, small(label)]
        .spacing(units(2))
        .align_y(Alignment::Center);
    if message.voice {
        line = line.push(icon(Icon::Mic, ICON_SIZE - 6.0, text_secondary()));
    }
    line = line.push(
        button(text(SPEED_LABELS[section.speed]).size(FONT_SIZE_SMALL))
            .padding([2.0, units(2)])
            .style(style::toggle(false))
            .on_press(Message::AudioSpeed),
    );
    match section.media(&message.id) {
        Some(Media::Failed(error)) => column![line, failure(error, Message::AudioToggle(id))]
            .spacing(units(1))
            .into(),
        _ => line.into(),
    }
}

fn document_view<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let id = message.id.clone();
    let name = if message.file_name.is_empty() {
        "Documento".to_string()
    } else {
        message.file_name.clone()
    };
    let risky = is_dangerous(&name);
    let action: Element<'a, Message> = match section.media(&message.id) {
        Some(Media::Ready(_, path)) => {
            let risky = risky || is_dangerous(&path.to_string_lossy());
            if risky {
                icon_label_button(Icon::Folder, "Abrir pasta", Some(Message::MediaReveal(id)))
                    .into()
            } else {
                icon_label_button(Icon::External, "Abrir", Some(Message::MediaOpen(id))).into()
            }
        }
        Some(Media::Loading) => small("Baixando").into(),
        Some(Media::Failed(error)) => failure(error, Message::MediaRequested(id)),
        None => {
            icon_label_button(Icon::Download, "Baixar", Some(Message::MediaRequested(id))).into()
        }
    };
    let mut details = column![body(name), small(format_size(message.size))].spacing(2);
    if risky {
        details =
            details.push(small("Arquivo executável: só é possível abrir a pasta").color(warning()));
    }
    container(
        row![
            icon(Icon::File, ICON_SIZE + 6.0, accent()),
            details.width(Length::Fill),
            action,
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding(units(2))
    .width(280)
    .style(style::elevated)
    .into()
}

fn info_card<'a>(message: &'a ChatMessage) -> Element<'a, Message> {
    let (glyph, label) = match message.kind {
        MessageKind::Contact => (Icon::User, "Contato"),
        MessageKind::Location => (Icon::MapPin, "Localização"),
        _ => (Icon::Poll, "Enquete"),
    };
    let mut details = column![body(label)].spacing(2);
    if !message.text.trim().is_empty() {
        details = details.push(small(message.text.as_str()));
    }
    row![icon(glyph, ICON_SIZE + 4.0, accent()), details]
        .spacing(units(3))
        .align_y(Alignment::Center)
        .into()
}

fn media_block<'a>(
    section: &'a WhatsApp,
    message: &'a ChatMessage,
) -> Option<Element<'a, Message>> {
    if message.deleted {
        return None;
    }
    match message.kind {
        MessageKind::Image if can_fetch(message) => Some(image_view(section, message)),
        MessageKind::Sticker if can_fetch(message) => Some(sticker_view(section, message)),
        MessageKind::Video => Some(video_view(section, message)),
        MessageKind::Audio => Some(audio_view(section, message)),
        MessageKind::Document => Some(document_view(section, message)),
        MessageKind::Contact | MessageKind::Location | MessageKind::Poll => {
            Some(info_card(message))
        }
        _ => None,
    }
}

fn quote<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let loaded = section.messages.iter().any(|m| m.id == message.quoted_id);
    let content = container(
        column![
            small(message.quoted_label()).color(WHATSAPP_GREEN),
            small(message.quoted_text.as_str()),
        ]
        .spacing(2),
    )
    .padding([units(1), units(2)])
    .style(style::outlined(accent(), true));
    button(content)
        .padding(0)
        .style(style::ghost)
        .on_press_maybe(
            loaded.then(|| Message::JumpTo(message.jid.clone(), message.quoted_id.clone())),
        )
        .into()
}

fn reaction_row<'a>(
    section: &'a WhatsApp,
    message: &'a ChatMessage,
) -> Option<Element<'a, Message>> {
    let list = section.reactions.get(&message.id)?;
    let groups = group_reactions(list);
    if groups.is_empty() {
        return None;
    }
    Some(
        groups
            .into_iter()
            .fold(row![].spacing(units(1)), |line, (emoji, count, mine)| {
                let label = if count > 1 {
                    format!("{emoji} {count}")
                } else {
                    emoji.clone()
                };
                line.push(
                    button(text(label).size(FONT_SIZE))
                        .padding([0.0, units(2)])
                        .style(style::toggle(mine))
                        .on_press(Message::ReactWith(message.id.clone(), emoji)),
                )
            })
            .wrap()
            .into(),
    )
}

pub fn bubble<'a>(
    section: &'a WhatsApp,
    message: &'a ChatMessage,
    group: bool,
) -> Element<'a, Message> {
    let mut content = Column::new().spacing(2);
    if group && !message.mine {
        content = content.push(small(message.sender_label()).color(WHATSAPP_GREEN));
    }
    if message.forwarded && !message.deleted {
        content = content.push(
            row![
                icon(Icon::Forward, ICON_SIZE - 8.0, text_secondary()),
                small("Encaminhada"),
            ]
            .spacing(units(1))
            .align_y(Alignment::Center),
        );
    }
    if !message.quoted_id.is_empty() && !message.deleted {
        content = content.push(quote(section, message));
    }
    let mut footer = row![].spacing(units(2)).align_y(Alignment::Center);
    if message.edited && !message.deleted {
        footer = footer.push(small("editada"));
    }
    footer = footer.push(small(message.at.format("%H:%M").to_string()));
    let mut footer = Some(footer);
    let reactions = reaction_row(section, message);
    match media_block(section, message) {
        Some(block) => {
            content = content.push(block);
            let caption = !message.text.trim().is_empty()
                && !matches!(
                    message.kind,
                    MessageKind::Contact | MessageKind::Location | MessageKind::Poll
                );
            if caption {
                content = content.push(body(message.text.as_str()));
            }
        }
        None if message.deleted => {
            content = content.push(muted(message.display_text()));
        }
        None => {
            let shown = message.display_text();
            let short = !shown.contains('\n') && shown.chars().count() <= INLINE_CHARS;
            let inline = if short && reactions.is_none() {
                footer.take()
            } else {
                None
            };
            match inline {
                Some(inline) => {
                    content = content.push(
                        row![body(shown), inline]
                            .spacing(units(3))
                            .align_y(Alignment::End),
                    );
                }
                None => {
                    content = content.push(body(shown));
                }
            }
        }
    }
    if let Some(reactions) = reactions {
        content = content.push(reactions);
    }
    if let Some(footer) = footer {
        let wide = message.text.lines().any(|l| l.chars().count() > WIDE_CHARS);
        content = if wide && !message.deleted {
            content.push(row![space().width(Length::Fill), footer])
        } else {
            content.push(footer)
        };
    }
    let highlighted = section.highlight.as_deref() == Some(message.id.as_str());
    let base = container(content)
        .padding([units(2), units(3)])
        .max_width(520);
    let bubble: Element<'a, Message> = if highlighted {
        base.style(style::outlined(warning(), false)).into()
    } else if message.mine {
        base.style(style::banner).into()
    } else {
        base.style(style::elevated).into()
    };
    let bubble = mouse_area(bubble).on_right_press(Message::MessageMenu(message.id.clone()));
    if message.mine {
        row![space().width(Length::Fill), bubble].into()
    } else {
        row![bubble, space().width(Length::Fill)].into()
    }
}
