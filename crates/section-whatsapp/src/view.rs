use hub_core::agenda::recurrence::weekday_short;
use hub_core::chrono::{Datelike, Local, NaiveDateTime};
use hub_core::whatsapp::{Conversation, Message as ChatMessage};
use hub_ui_kit::iced::widget::{
    Column, button, column, container, image, mouse_area, qr_code, row, scrollable, space, text,
};
use hub_ui_kit::iced::{Alignment, Color, Element, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::{Connection, Media, Message, VISIBLE_CONVERSATIONS, WhatsApp};

const LIST_WIDTH: f32 = 300.0;
const IMAGE_WIDTH: f32 = 320.0;
const WHATSAPP_GREEN: Color = Color::from_rgb8(0x4F, 0xC3, 0x7A);

fn short_time(at: NaiveDateTime) -> String {
    let today = Local::now().date_naive();
    match (today - at.date()).num_days() {
        0 => at.format("%H:%M").to_string(),
        1 => "ontem".to_string(),
        2..=6 => weekday_short(at.weekday()).to_string(),
        _ => at.format("%d/%m").to_string(),
    }
}

fn status_line(section: &WhatsApp) -> Element<'_, Message> {
    let (label, color) = match section.connection() {
        Connection::Starting => ("Iniciando".to_string(), text_secondary()),
        Connection::WaitingQr => ("Aguardando pareamento".to_string(), warning()),
        Connection::Qr(_) => ("Escaneie o QR code".to_string(), warning()),
        Connection::Connected(_) => ("Conectado".to_string(), WHATSAPP_GREEN),
        Connection::Offline(_) => ("Desconectado".to_string(), danger()),
    };
    let status = row![
        container(space())
            .width(8)
            .height(8)
            .style(style::dot(color)),
        small(label).color(color),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center);
    if section.is_running() {
        row![
            status,
            space().width(Length::Fill),
            ghost_button("Desligar", Some(Message::SwitchOff)),
        ]
        .align_y(Alignment::Center)
        .into()
    } else {
        status.into()
    }
}

fn conversation_item<'a>(
    conversation: &'a Conversation,
    selected: bool,
    target: bool,
) -> Element<'a, Message> {
    let avatar_icon = if conversation.group {
        Icon::Users
    } else {
        Icon::User
    };
    let time = conversation.last_at.map(short_time).unwrap_or_default();
    let mut meta = row![].spacing(units(1)).align_y(Alignment::Center);
    if conversation.pinned {
        meta = meta.push(icon(Icon::Flag, ICON_SIZE - 6.0, text_secondary()));
    }
    let content = mouse_area(
        row![
            container(icon(avatar_icon, ICON_SIZE - 2.0, WHATSAPP_GREEN))
                .padding(units(2))
                .style(style::badge(WHATSAPP_GREEN)),
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
                container(
                    text(conversation.preview.clone().unwrap_or_default())
                        .size(FONT_SIZE_SMALL)
                        .color(text_secondary())
                        .wrapping(text::Wrapping::None)
                )
                .clip(true)
                .width(Length::Fill),
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

fn list(section: &WhatsApp) -> Element<'_, Message> {
    let selected = section.selected().map(|c| c.jid.clone());
    let items = section.conversations();
    let total = items.len();
    let body: Element<'_, Message> = if items.is_empty() {
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
        scroll(column)
    };
    let archived = section.archived_count();
    let footer: Element<'_, Message> = if section.showing_archived() {
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
            title("WhatsApp"),
            status_line(section),
            text_field("Buscar conversa", section.query(), Message::QueryChanged),
            container(body).height(Length::Fill),
            footer,
        ]
        .spacing(units(3)),
    )
    .width(LIST_WIDTH)
    .height(Length::Fill)
    .into()
}

fn media_view<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let open = Message::MediaRequested(message.id.clone());
    match section.media(&message.id) {
        Some(Media::Ready(handle, _)) => mouse_area(image(handle.clone()).width(IMAGE_WIDTH))
            .on_press(open)
            .interaction(hub_ui_kit::iced::mouse::Interaction::Pointer)
            .into(),
        Some(Media::Loading) => small("Baixando imagem").into(),
        Some(Media::Failed(error)) => column![
            small(error.as_str()).color(danger()),
            icon_label_button(Icon::Refresh, "Tentar de novo", Some(open)),
        ]
        .spacing(units(1))
        .into(),
        None => icon_label_button(Icon::Image, "Ver imagem", Some(open)).into(),
    }
}

fn bubble<'a>(
    section: &'a WhatsApp,
    message: &'a ChatMessage,
    group: bool,
) -> Element<'a, Message> {
    let mut content = Column::new().spacing(2);
    if group && !message.mine {
        content = content.push(small(message.sender_label()).color(WHATSAPP_GREEN));
    }
    if message.has_media() {
        content = content.push(media_view(section, message));
        if !message.text.trim().is_empty() {
            content = content.push(body(message.text.as_str()));
        }
    } else {
        content = content.push(body(message.display_text()));
    }
    content = content
        .push(container(small(message.at.format("%H:%M").to_string())).align_right(Length::Shrink));
    let bubble = container(content)
        .padding([units(2), units(3)])
        .max_width(520)
        .style(if message.mine {
            style::banner
        } else {
            style::elevated
        });
    if message.mine {
        row![space().width(Length::Fill), bubble].into()
    } else {
        row![bubble, space().width(Length::Fill)].into()
    }
}

fn pairing(section: &WhatsApp) -> Element<'_, Message> {
    let content: Element<'_, Message> = match section.connection() {
        Connection::Qr(data) => column![
            heading("Vincule o WhatsApp"),
            column![
                body("1. Abra o WhatsApp no celular."),
                body("2. Toque em Configurações (iPhone) ou nos três pontos (Android)."),
                body("3. Entre em Dispositivos conectados e toque em Conectar um dispositivo."),
                body("4. Aponte a câmera para o código abaixo."),
            ]
            .spacing(units(1)),
            small("O código muda a cada poucos segundos. A sessão fica salva neste computador."),
            container(qr_code(data).cell_size(5))
                .padding(units(3))
                .style(|_| container::Style {
                    background: Some(hub_ui_kit::iced::Background::Color(Color::WHITE)),
                    border: hub_ui_kit::iced::Border {
                        radius: RADIUS_PANEL.into(),
                        ..Default::default()
                    },
                    ..container::Style::default()
                }),
        ]
        .spacing(units(3))
        .align_x(Alignment::Center)
        .into(),
        Connection::Offline(reason) if reason.starts_with(crate::SIDECAR_MISSING) => column![
            icon(Icon::Warning, 28.0, warning()),
            heading("Falta o componente do WhatsApp"),
            muted("O hub conversa com o WhatsApp por um programa auxiliar chamado whatsapp-bridge, escrito em Go. Ele precisa estar na mesma pasta do hub."),
            column![
                body("1. Instale o Go: winget install GoLang.Go"),
                body("2. Na pasta do projeto, entre em sidecars/whatsapp-bridge"),
                body("3. Compile: go build -o whatsapp-bridge.exe ."),
                body("4. Copie o whatsapp-bridge.exe para a pasta do hub.exe"),
            ]
            .spacing(units(1)),
            small(reason.as_str()),
            primary_icon_button(Icon::Refresh, "Tentar de novo", Some(Message::Retry)),
        ]
        .spacing(units(3))
        .max_width(560)
        .align_x(Alignment::Center)
        .into(),
        Connection::Offline(reason) => column![
            icon(Icon::Warning, 28.0, warning()),
            heading("WhatsApp desconectado"),
            muted(reason.as_str()),
            primary_icon_button(Icon::Refresh, "Tentar de novo", Some(Message::Retry)),
        ]
        .spacing(units(3))
        .align_x(Alignment::Center)
        .into(),
        _ => column![
            icon(Icon::WhatsApp, 28.0, WHATSAPP_GREEN),
            muted("Conectando ao WhatsApp"),
        ]
        .spacing(units(3))
        .align_x(Alignment::Center)
        .into(),
    };
    container(content)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

fn chat(section: &WhatsApp) -> Element<'_, Message> {
    let Some(conversation) = section.selected() else {
        return if section.is_connected() {
            container(muted("Escolha uma conversa"))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into()
        } else {
            pairing(section)
        };
    };
    let header: Element<'_, Message> = match section.alias() {
        Some(alias) => row![
            text_field("Apelido (vazio remove)", alias, Message::AliasChanged)
                .on_submit(Message::AliasSaved),
            ghost_button("Cancelar", Some(Message::AliasCancelled)),
            primary_button("Salvar", Some(Message::AliasSaved)),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center)
        .into(),
        None => {
            let mut info = column![heading(conversation.title())].spacing(2);
            if conversation.alias.is_some() && !conversation.name.is_empty() {
                info = info.push(small(conversation.name.as_str()));
            } else if conversation.group {
                info = info.push(small("Grupo"));
            }
            row![
                info.width(Length::Fill),
                icon_label_button(Icon::Edit, "Apelido", Some(Message::AliasEdit)),
            ]
            .align_y(Alignment::Center)
            .into()
        }
    };
    let messages: Element<'_, Message> = if section.messages().is_empty() {
        container(muted("Sem mensagens guardadas nesta conversa."))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    } else {
        scrollable(
            container(
                section
                    .messages()
                    .iter()
                    .fold(Column::new().spacing(units(2)), |col, m| {
                        col.push(bubble(section, m, conversation.group))
                    }),
            )
            .padding(Padding::default().right(units(3))),
        )
        .anchor_bottom()
        .height(Length::Fill)
        .style(style::scroll)
        .into()
    };
    let can_send = section.is_connected() && !section.sending();
    let composer: Element<'_, Message> = if section.is_connected() {
        row![
            button(icon(Icon::File, ICON_SIZE - 2.0, text_secondary()))
                .padding(units(2))
                .style(style::ghost)
                .on_press_maybe(can_send.then_some(Message::AttachFiles)),
            text_field("Mensagem", section.draft(), Message::DraftChanged)
                .id(crate::DRAFT_INPUT)
                .on_submit(Message::Send),
            button(icon(Icon::Send, ICON_SIZE - 2.0, on_accent()))
                .padding(units(2))
                .style(style::primary)
                .on_press_maybe(
                    (can_send && !section.draft().trim().is_empty()).then_some(Message::Send)
                ),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center)
        .into()
    } else {
        small("Conecte o WhatsApp para responder.").into()
    };
    let suggestions = section.mention_suggestions();
    let mut bottom = Column::new().spacing(units(1));
    if !suggestions.is_empty() {
        bottom = bottom.push(
            container(suggestions.into_iter().fold(
                Column::new().spacing(2),
                |col, (name, jid)| {
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
                },
            ))
            .padding(units(1))
            .width(Length::Fill)
            .style(style::elevated),
        );
    }
    let bottom = bottom.push(composer);
    column![header, messages, bottom]
        .spacing(units(3))
        .height(Length::Fill)
        .into()
}

pub fn view(section: &WhatsApp) -> Element<'_, Message> {
    let mut right = Column::new().spacing(units(3)).height(Length::Fill);
    if let Some(error) = section.error() {
        right = right.push(
            container(
                row![
                    body(error).color(danger()).width(Length::Fill),
                    icon_button(Icon::Close, text_secondary(), Some(Message::ErrorDismissed)),
                ]
                .align_y(Alignment::Center),
            )
            .padding([units(1), units(3)])
            .style(style::elevated),
        );
    }
    right = right.push(chat(section));
    panel(
        row![
            list(section),
            container(right)
                .padding(Padding::default().left(units(4)))
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .height(Length::Fill),
    )
    .into()
}
