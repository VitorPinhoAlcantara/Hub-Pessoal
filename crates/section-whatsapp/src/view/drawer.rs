use hub_core::whatsapp::{Message as ChatMessage, MessageKind};
use hub_ui_kit::iced::widget::{Column, button, column, container, image, row, text};
use hub_ui_kit::iced::{Alignment, ContentFit, Element, Length};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use super::clip_text;
use crate::model::*;
use crate::{Message, WhatsApp};

const DRAWER_WIDTH: f32 = 300.0;
const TILE: f32 = 88.0;
const TILES_PER_ROW: usize = 3;
const MAX_ITEMS: usize = 150;

fn jump(section: &WhatsApp, message: &ChatMessage) -> Option<Message> {
    section
        .selected
        .clone()
        .map(|jid| Message::JumpTo(jid, message.id.clone()))
}

fn tile<'a>(section: &'a WhatsApp, message: &'a ChatMessage) -> Element<'a, Message> {
    let content: Element<'a, Message> = match section.media(&message.id) {
        Some(Media::Ready(Some(handle), _)) => image(handle.clone())
            .width(TILE)
            .height(TILE)
            .content_fit(ContentFit::Cover)
            .into(),
        _ => {
            let glyph = if message.kind == MessageKind::Video {
                Icon::Video
            } else {
                Icon::Image
            };
            container(icon(glyph, ICON_SIZE + 4.0, text_secondary()))
                .width(TILE)
                .height(TILE)
                .center_x(TILE)
                .center_y(TILE)
                .style(style::elevated)
                .into()
        }
    };
    button(content)
        .padding(0)
        .style(style::ghost)
        .on_press_maybe(jump(section, message))
        .into()
}

fn row_item<'a>(
    section: &'a WhatsApp,
    message: &'a ChatMessage,
    glyph: Icon,
    label: String,
    detail: String,
) -> Element<'a, Message> {
    button(
        row![
            icon(glyph, ICON_SIZE, accent()),
            column![
                container(text(label).size(FONT_SIZE).wrapping(text::Wrapping::None))
                    .clip(true)
                    .width(Length::Fill),
                clip_text(detail),
            ]
            .spacing(2)
            .width(Length::Fill),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
    )
    .padding([units(1), units(2)])
    .width(Length::Fill)
    .style(style::ghost)
    .on_press_maybe(jump(section, message))
    .into()
}

pub fn drawer(section: &WhatsApp) -> Option<Element<'_, Message>> {
    let state = section.drawer.as_ref()?;
    let tabs = row![
        tab_button(
            Icon::Image,
            "Mídia",
            state.tab == DrawerTab::Media,
            Message::DrawerTab(DrawerTab::Media)
        ),
        tab_button(
            Icon::File,
            "Documentos",
            state.tab == DrawerTab::Documents,
            Message::DrawerTab(DrawerTab::Documents)
        ),
        tab_button(
            Icon::Mic,
            "Áudios",
            state.tab == DrawerTab::Audios,
            Message::DrawerTab(DrawerTab::Audios)
        ),
    ]
    .spacing(units(1));
    let items: Vec<&ChatMessage> = state
        .items
        .iter()
        .filter(|m| state.tab.accepts(m))
        .take(MAX_ITEMS)
        .collect();
    let body: Element<'_, Message> = if items.is_empty() {
        container(muted(if state.loading {
            "Carregando"
        } else {
            "Nenhum item"
        }))
        .padding(units(3))
        .into()
    } else {
        match state.tab {
            DrawerTab::Media => {
                let mut grid = Column::new().spacing(units(1));
                for chunk in items.chunks(TILES_PER_ROW) {
                    grid = grid.push(chunk.iter().fold(row![].spacing(units(1)), |line, m| {
                        line.push(tile(section, m))
                    }));
                }
                scroll(grid)
            }
            DrawerTab::Documents => {
                scroll(items.into_iter().fold(Column::new().spacing(2), |col, m| {
                    let name = if m.file_name.is_empty() {
                        "Documento".to_string()
                    } else {
                        m.file_name.clone()
                    };
                    col.push(row_item(
                        section,
                        m,
                        Icon::File,
                        name,
                        format!("{} {}", format_size(m.size), m.at.format("%d/%m/%Y")),
                    ))
                }))
            }
            DrawerTab::Audios => {
                scroll(items.into_iter().fold(Column::new().spacing(2), |col, m| {
                    let author = if m.mine {
                        "Você".to_string()
                    } else {
                        m.sender_label()
                    };
                    col.push(row_item(
                        section,
                        m,
                        Icon::Mic,
                        format!("{author} {}", format_clock(m.duration as u64)),
                        m.at.format("%d/%m/%Y %H:%M").to_string(),
                    ))
                }))
            }
        }
    };
    Some(
        container(
            column![
                row![
                    heading("Arquivos"),
                    hub_ui_kit::iced::widget::space().width(Length::Fill),
                    icon_button(Icon::Close, text_secondary(), Some(Message::DrawerToggle)),
                ]
                .align_y(Alignment::Center),
                tabs,
                container(body).height(Length::Fill),
            ]
            .spacing(units(3)),
        )
        .width(DRAWER_WIDTH)
        .height(Length::Fill)
        .into(),
    )
}
