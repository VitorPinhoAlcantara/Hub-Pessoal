mod bubble;
mod composer;
mod dialogs;
mod drawer;
mod list;

use hub_core::chrono::{Local, NaiveDate};
use hub_ui_kit::iced::widget::{Column, Id, column, container, row, scrollable, space, text};
use hub_ui_kit::iced::{Alignment, Color, Element, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::model::*;
use crate::{Connection, Message, SEARCH_INPUT, WhatsApp};

const PREVIEW_CHARS: usize = 120;

pub const WHATSAPP_GREEN: Color = Color::from_rgb8(0x4F, 0xC3, 0x7A);

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
            container(hub_ui_kit::iced::widget::qr_code(data).cell_size(5))
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

fn header(section: &WhatsApp) -> Element<'_, Message> {
    let Some(conversation) = section.selected() else {
        return space().into();
    };
    if let Some(alias) = section.alias() {
        return row![
            text_field("Apelido (vazio remove)", alias, Message::AliasChanged)
                .on_submit(Message::AliasSaved),
            ghost_button("Cancelar", Some(Message::AliasCancelled)),
            primary_button("Salvar", Some(Message::AliasSaved)),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center)
        .into();
    }
    let mut info = column![heading(conversation.title())].spacing(2);
    if conversation.alias.is_some() && !conversation.name.is_empty() {
        info = info.push(small(conversation.name.as_str()));
    } else if conversation.group {
        info = info.push(small("Grupo"));
    }
    row![
        avatar(
            section.photo_of(&conversation.jid),
            &conversation.title(),
            36.0,
            WHATSAPP_GREEN
        ),
        info.width(Length::Fill),
        with_tooltip(
            icon_button(
                Icon::Search,
                if section.search.is_some() {
                    accent()
                } else {
                    text_secondary()
                },
                Some(Message::SearchToggle)
            ),
            "Buscar na conversa",
            hub_ui_kit::iced::widget::tooltip::Position::Bottom,
        ),
        with_tooltip(
            icon_button(
                Icon::SidePanel,
                if section.drawer.is_some() {
                    accent()
                } else {
                    text_secondary()
                },
                Some(Message::DrawerToggle)
            ),
            "Mídia, documentos e áudios",
            hub_ui_kit::iced::widget::tooltip::Position::Bottom,
        ),
        icon_label_button(Icon::Edit, "Apelido", Some(Message::AliasEdit)),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center)
    .into()
}

fn search_bar(section: &WhatsApp) -> Option<Element<'_, Message>> {
    let state = section.search.as_ref()?;
    let status: String = if state.loading {
        "Buscando".to_string()
    } else if state.query.trim().is_empty() {
        String::new()
    } else if state.results.is_empty() {
        "Nada encontrado".to_string()
    } else {
        format!("{} de {}", state.index + 1, state.results.len())
    };
    Some(
        row![
            text_field(
                "Buscar nesta conversa",
                &state.query,
                Message::SearchChanged
            )
            .id(SEARCH_INPUT)
            .on_submit(Message::SearchNext),
            small(status),
            icon_button(
                Icon::ChevronUp,
                text_secondary(),
                (!state.results.is_empty()).then_some(Message::SearchNext)
            ),
            icon_button(
                Icon::ChevronDown,
                text_secondary(),
                (!state.results.is_empty()).then_some(Message::SearchPrevious)
            ),
            icon_button(Icon::Close, text_secondary(), Some(Message::SearchToggle)),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center)
        .into(),
    )
}

fn day_separator<'a>(date: NaiveDate) -> Element<'a, Message> {
    container(
        container(small(day_label(date, Local::now().date_naive())))
            .padding([2.0, units(3)])
            .style(style::elevated),
    )
    .center_x(Length::Fill)
    .into()
}

fn older_control(section: &WhatsApp) -> Element<'_, Message> {
    let range = section.visible();
    let mut stack = Column::new().spacing(units(1)).align_x(Alignment::Center);
    if section.history_wait.is_some() {
        stack = stack.push(small("Aguardando o celular"));
    } else if range.start > 0 {
        stack = stack.push(icon_label_button(
            Icon::ChevronUp,
            "Mostrar mensagens anteriores",
            Some(Message::LoadOlder),
        ));
    } else {
        stack = stack.push(icon_label_button(
            Icon::Refresh,
            "Carregar mensagens anteriores",
            Some(Message::LoadOlder),
        ));
    }
    if let Some(notice) = &section.history_notice {
        stack = stack.push(small(notice.as_str()));
    }
    container(stack).center_x(Length::Fill).into()
}

fn message_list(section: &WhatsApp, group: bool) -> Element<'_, Message> {
    if section.messages.is_empty() {
        return container(muted("Sem mensagens guardadas nesta conversa."))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    }
    let range = section.visible();
    let mut list = Column::new().spacing(units(2));
    list = list.push(older_control(section));
    let mut last_day: Option<NaiveDate> = None;
    for message in &section.messages[range] {
        let day = message.at.date();
        if last_day != Some(day) {
            list = list.push(day_separator(day));
            last_day = Some(day);
        }
        list = list.push(bubble::bubble(section, message, group));
    }
    if section.view_end.is_some() {
        list = list.push(
            container(icon_label_button(
                Icon::ChevronDown,
                "Ir para as mensagens recentes",
                Some(Message::BackToRecent),
            ))
            .center_x(Length::Fill),
        );
    }
    scrollable(container(list).padding(Padding::default().right(units(3))))
        .id(Id::new(CHAT_SCROLL))
        .anchor_bottom()
        .height(Length::Fill)
        .style(style::scroll)
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
    let mut main = Column::new().spacing(units(3)).height(Length::Fill);
    main = main.push(header(section));
    if let Some(bar) = search_bar(section) {
        main = main.push(bar);
    }
    main = main.push(message_list(section, conversation.group));
    main = main.push(composer::composer(section));
    let main: Element<'_, Message> = main.width(Length::Fill).into();
    match drawer::drawer(section) {
        Some(side) => row![main, side].spacing(units(3)).into(),
        None => main,
    }
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
    let base = panel(
        row![
            list::list(section),
            container(right)
                .padding(Padding::default().left(units(4)))
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .height(Length::Fill),
    );
    dialogs::overlay(section, base.into())
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .take(PREVIEW_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn clip_text<'a>(value: impl AsRef<str>) -> Element<'a, Message> {
    container(
        text(single_line(value.as_ref()))
            .size(FONT_SIZE_SMALL)
            .color(text_secondary())
            .wrapping(hub_ui_kit::iced::widget::text::Wrapping::None),
    )
    .clip(true)
    .width(Length::Fill)
    .into()
}
