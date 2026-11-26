use hub_core::agenda::recurrence::weekday_short;
use hub_core::chrono::{Datelike, Local, NaiveDateTime};
use hub_core::whatsapp::{Conversation, Message as ChatMessage};
use hub_ui_kit::iced::widget::{Column, button, column, container, mouse_area, row, space, text};
use hub_ui_kit::iced::{Alignment, Length};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use super::{WHATSAPP_GREEN, clip_text};
use crate::{Connection, Message, VISIBLE_CONVERSATIONS, WhatsApp};

const LIST_WIDTH: f32 = 320.0;
const AVATAR: f32 = 38.0;

fn short_time(at: NaiveDateTime) -> String {
    let today = Local::now().date_naive();
    match (today - at.date()).num_days() {
        0 => at.format("%H:%M").to_string(),
        1 => "ontem".to_string(),
        2..=6 => weekday_short(at.weekday()).to_string(),
        _ => at.format("%d/%m").to_string(),
    }
}

fn status_line(section: &WhatsApp) -> hub_ui_kit::iced::Element<'_, Message> {
    let (label, color) = match section.connection() {
        Connection::Starting => ("Iniciando".to_string(), text_secondary()),
        Connection::WaitingQr => ("Aguardando pareamento".to_string(), warning()),
        Connection::Qr(_) => ("Escaneie o QR code".to_string(), warning()),
        Connection::Connected(_) => ("Conectado".to_string(), WHATSAPP_GREEN),
        Connection::Offline(_) => ("Desconectado".to_string(), danger()),
    };
    let content: hub_ui_kit::iced::Element<'_, Message> =
        if section.status_hover && section.is_running() {
            button(small("Desligar").color(danger()))
                .padding([units(1), units(2)])
                .style(style::ghost)
                .on_press(Message::SwitchOff)
                .into()
        } else {
            container(
                row![
                    container(space())
                        .width(8)
                        .height(8)
                        .style(style::dot(color)),
                    small(label).color(color),
                ]
                .spacing(units(2))
                .align_y(Alignment::Center),
            )
            .padding([units(1), units(2)])
            .into()
        };
    mouse_area(content)
        .on_enter(Message::StatusHovered(true))
        .on_exit(Message::StatusHovered(false))
        .into()
}

fn conversation_item<'a>(
    section: &'a WhatsApp,
    conversation: &'a Conversation,
    selected: bool,
    target: bool,
) -> hub_ui_kit::iced::Element<'a, Message> {
    let time = conversation.last_at.map(short_time).unwrap_or_default();
    let mut meta = row![].spacing(units(1)).align_y(Alignment::Center);
    if conversation.pinned {
        meta = meta.push(icon(Icon::Flag, ICON_SIZE - 6.0, text_secondary()));
    }
    let content = mouse_area(
        row![
            avatar(
                section.photo_of(&conversation.jid),
                &conversation.title(),
                AVATAR,
                WHATSAPP_GREEN
            ),
            column![
                row![
                    container(
                        text(conversation.title())
                            .size(FONT_SIZE)
                            .wrapping(text::Wrapping::None)
                    )
                    .clip(true)
                    .width(Length::Fill),
                    meta.push(small(time)),
                ]
                .spacing(units(2)),
                clip_text(conversation.preview.clone().unwrap_or_default()),
            ]
            .spacing(2)
            .width(Length::Fill),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .on_press(Message::ConversationPressed(conversation.jid.clone()));
    let item = button(content)
        .padding([units(2), units(3)])
        .width(Length::Fill)
        .style(style::selectable(selected || target))
        .on_press(Message::Selected(conversation.jid.clone()));
    mouse_area(item)
        .on_right_press(Message::ConversationMenu(conversation.jid.clone()))
        .on_enter(Message::ConversationHovered(conversation.jid.clone()))
        .on_exit(Message::ConversationLeft(conversation.jid.clone()))
        .into()
}

fn hit_item<'a>(
    section: &'a WhatsApp,
    message: &'a ChatMessage,
) -> hub_ui_kit::iced::Element<'a, Message> {
    let title = section
        .conversations
        .iter()
        .find(|c| c.jid == message.jid)
        .map_or_else(String::new, Conversation::title);
    let author = if message.mine {
        "Você".to_string()
    } else {
        message.sender_label()
    };
    button(
        column![
            row![
                container(text(title).size(FONT_SIZE).wrapping(text::Wrapping::None))
                    .clip(true)
                    .width(Length::Fill),
                small(short_time(message.at)),
            ]
            .spacing(units(2)),
            clip_text(format!("{author}: {}", message.display_text())),
        ]
        .spacing(2),
    )
    .padding([units(2), units(3)])
    .width(Length::Fill)
    .style(style::ghost)
    .on_press(Message::JumpTo(message.jid.clone(), message.id.clone()))
    .into()
}

pub fn list(section: &WhatsApp) -> hub_ui_kit::iced::Element<'_, Message> {
    let selected = section.selected().map(|c| c.jid.clone());
    let items = section.conversations();
    let total = items.len();
    let hits = &section.global_hits;
    let body: hub_ui_kit::iced::Element<'_, Message> = if items.is_empty() && hits.is_empty() {
        container(muted(if section.query().is_empty() {
            "Nenhuma conversa ainda. Elas aparecem conforme as mensagens chegam."
        } else {
            "Nada encontrado."
        }))
        .padding(units(4))
        .into()
    } else {
        let mut column = items.into_iter().take(VISIBLE_CONVERSATIONS).fold(
            Column::new().spacing(2),
            |col, c| {
                col.push(conversation_item(
                    section,
                    c,
                    selected.as_deref() == Some(c.jid.as_str()),
                    section.drop_target(&c.jid),
                ))
            },
        );
        if total > VISIBLE_CONVERSATIONS {
            column = column.push(
                container(small(format!(
                    "Mostrando {VISIBLE_CONVERSATIONS} de {total}. Use a busca para achar as outras."
                )))
                .padding(units(2)),
            );
        }
        if !hits.is_empty() {
            column = column.push(container(small("Mensagens")).padding([units(2), units(3)]));
            column = hits
                .iter()
                .fold(column, |col, message| col.push(hit_item(section, message)));
        }
        scroll(column)
    };
    let archived = section.archived_count();
    let footer: hub_ui_kit::iced::Element<'_, Message> = if section.showing_archived() {
        icon_label_button(
            Icon::List,
            "Voltar às conversas",
            Some(Message::ArchivedToggled),
        )
        .into()
    } else if archived > 0 {
        button(text(format!("Arquivadas ({archived})")).size(FONT_SIZE))
            .padding([units(2), units(3)])
            .style(style::ghost)
            .on_press(Message::ArchivedToggled)
            .into()
    } else {
        space().into()
    };
    container(
        column![
            row![
                title("WhatsApp"),
                space().width(Length::Fill),
                status_line(section)
            ]
            .align_y(Alignment::Center),
            text_field(
                "Buscar conversa ou mensagem",
                section.query(),
                Message::QueryChanged
            ),
            container(body).height(Length::Fill),
            footer,
        ]
        .spacing(units(3)),
    )
    .width(LIST_WIDTH)
    .height(Length::Fill)
    .into()
}
