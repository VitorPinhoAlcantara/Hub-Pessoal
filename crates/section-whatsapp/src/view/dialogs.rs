use hub_core::whatsapp::{Sticker, StickerSection};
use hub_ui_kit::iced::widget::{
    Column, button, column, container, hover, image, row, scrollable, space, stack, text,
};
use hub_ui_kit::iced::{Alignment, ContentFit, Element, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use super::WHATSAPP_GREEN;
use crate::emoji::{CATEGORIES, search};
use crate::model::*;
use crate::{Message, WhatsApp};

const GRID_COLUMNS: usize = 8;
const PICKER_HEIGHT: f32 = 220.0;
const STICKER_COLUMNS: usize = 6;
const STICKER_CELL: f32 = 64.0;
const STICKER_BOX: f32 = STICKER_CELL + 8.0;
pub const PICKER_WIDTH: f32 = 500.0;

fn emoji_button<'a>(emoji: &'static str) -> Element<'a, Message> {
    button(text(emoji).size(20))
        .padding(units(1))
        .width(36)
        .style(style::ghost)
        .on_press(Message::EmojiPicked(emoji.to_string()))
        .into()
}

fn sticker_cell<'a>(section: &'a WhatsApp, sticker: &'a Sticker) -> Element<'a, Message> {
    let preview: Element<'a, Message> = match section.sticker_previews.get(&sticker.hash) {
        Some(Some(handle)) => image(handle.clone())
            .width(STICKER_CELL)
            .height(STICKER_CELL)
            .content_fit(ContentFit::Contain)
            .into(),
        _ => container(small("..."))
            .center_x(STICKER_CELL)
            .center_y(STICKER_CELL)
            .into(),
    };
    let base = button(preview)
        .padding(4.0)
        .width(STICKER_BOX)
        .height(STICKER_BOX)
        .style(style::ghost)
        .on_press(Message::SendSticker(
            sticker.hash.clone(),
            sticker.action.clone(),
        ));
    let favorite = section.is_favorite(&sticker.hash);
    let star = container(
        container(
            button(icon(
                Icon::Star,
                ICON_SIZE - 6.0,
                if favorite {
                    warning()
                } else {
                    text_secondary()
                },
            ))
            .padding(2.0)
            .style(style::ghost)
            .on_press(Message::ToggleFavorite(
                sticker.hash.clone(),
                sticker.action.clone(),
            )),
        )
        .style(style::elevated),
    )
    .align_right(Length::Fill);
    let cell: Element<'a, Message> = if favorite {
        stack![base, star].into()
    } else {
        hover(base, star)
    };
    container(cell)
        .width(STICKER_BOX)
        .height(STICKER_BOX)
        .into()
}

fn sticker_panel(section: &WhatsApp) -> Element<'_, Message> {
    let current = section.emoji.section;
    let chips = [
        (StickerSection::Recent, "Recentes"),
        (StickerSection::Favorite, "Favoritas"),
        (StickerSection::Received, "Recebidas"),
    ]
    .into_iter()
    .fold(row![].spacing(units(1)), |line, (kind, label)| {
        line.push(
            button(text(label).size(FONT_SIZE_SMALL))
                .padding([2.0, units(3)])
                .style(style::toggle(current == kind))
                .on_press(Message::StickerSectionPicked(kind)),
        )
    });
    let add: Element<'_, Message> = button(
        column![icon(Icon::Plus, ICON_SIZE, accent()), small("Imagem do PC"),]
            .spacing(2)
            .align_x(Alignment::Center),
    )
    .padding(4.0)
    .width(STICKER_BOX)
    .height(STICKER_BOX)
    .style(style::ghost)
    .on_press(Message::StickerFile)
    .into();
    let mut cells: Vec<Element<'_, Message>> = vec![add];
    if let Some(list) = &section.stickers {
        cells.extend(
            list.iter()
                .filter(|sticker| !section.sticker_failed.contains(&sticker.hash))
                .map(|sticker| sticker_cell(section, sticker)),
        );
    }
    let mut grid = Column::new().spacing(units(1));
    let mut cells = cells.into_iter().peekable();
    while cells.peek().is_some() {
        let line = cells
            .by_ref()
            .take(STICKER_COLUMNS)
            .fold(row![].spacing(units(1)), |line, cell| line.push(cell));
        grid = grid.push(line);
    }
    let note: Option<Element<'_, Message>> = match &section.stickers {
        None => Some(small("Carregando figurinhas").into()),
        Some(list) if list.is_empty() => Some(
            small(match current {
                StickerSection::Recent => "As figurinhas que você enviar aparecem aqui",
                StickerSection::Favorite => {
                    "Passe o mouse numa figurinha e toque na estrela para salvar aqui"
                }
                StickerSection::Received => "Nenhuma figurinha recebida ainda",
            })
            .into(),
        ),
        Some(_) => None,
    };
    let mut content = column![grid]
        .spacing(units(2))
        .padding(Padding::default().right(units(4)));
    if let Some(note) = note {
        content = content.push(note);
    }
    column![
        chips,
        scrollable(content)
            .height(PICKER_HEIGHT)
            .style(style::scroll),
    ]
    .spacing(units(2))
    .into()
}

fn footer(stickers: bool) -> Element<'static, Message> {
    let tab = |glyph: Icon, active: bool, message: Message| {
        let color = if active { accent() } else { text_secondary() };
        button(container(icon(glyph, ICON_SIZE - 2.0, color)).center_x(Length::Fill))
            .padding([units(1), units(2)])
            .width(Length::Fill)
            .style(style::toggle(active))
            .on_press(message)
    };
    row![
        tab(Icon::Smile, !stickers, Message::EmojiCategory(0)),
        tab(Icon::Image, stickers, Message::StickerTab),
    ]
    .spacing(units(1))
    .into()
}

pub fn emoji_picker(section: &WhatsApp) -> Element<'_, Message> {
    let query = section.emoji.query.as_str();
    let reacting = matches!(section.dialog, Some(Dialog::React(_)));
    let stickers = section.emoji.stickers && !reacting;
    if stickers {
        return column![sticker_panel(section), footer(true)]
            .spacing(units(2))
            .width(Length::Fill)
            .into();
    }
    let tabs =
        CATEGORIES
            .iter()
            .enumerate()
            .fold(row![].spacing(units(1)), |line, (index, category)| {
                line.push(
                    button(text(category.icon).size(16))
                        .padding([2.0, units(2)])
                        .style(style::toggle(
                            query.is_empty() && section.emoji.category == index,
                        ))
                        .on_press(Message::EmojiCategory(index)),
                )
            });
    let items: Vec<&'static str> = if query.trim().is_empty() {
        CATEGORIES
            .get(section.emoji.category)
            .map(|c| c.items.iter().map(|(emoji, _)| *emoji).collect())
            .unwrap_or_default()
    } else {
        search(query)
    };
    let body: Element<'_, Message> = if items.is_empty() {
        container(muted("Nenhum emoji encontrado"))
            .padding(units(3))
            .into()
    } else {
        let mut grid = Column::new().spacing(units(1));
        for chunk in items.chunks(GRID_COLUMNS) {
            grid = grid.push(chunk.iter().fold(row![].spacing(units(1)), |line, emoji| {
                line.push(emoji_button(emoji))
            }));
        }
        scrollable(grid)
            .height(PICKER_HEIGHT)
            .style(style::scroll)
            .into()
    };
    let title: Element<'_, Message> = match CATEGORIES.get(section.emoji.category) {
        Some(category) if query.trim().is_empty() => small(category.name).into(),
        _ => small("Resultados").into(),
    };
    let mut picker = column![
        text_field("Buscar emoji", query, Message::EmojiQuery),
        tabs,
        title,
        body,
    ]
    .spacing(units(2))
    .width(Length::Fill);
    if !reacting {
        picker = picker.push(footer(false));
    }
    picker.into()
}

fn react_dialog<'a>(section: &'a WhatsApp, id: &str) -> Element<'a, Message> {
    let quick = QUICK_REACTIONS
        .iter()
        .fold(row![].spacing(units(1)), |line, emoji| {
            line.push(
                button(text(*emoji).size(26))
                    .padding(units(2))
                    .style(style::ghost)
                    .on_press(Message::ReactWith(id.to_string(), emoji.to_string())),
            )
        });
    column![
        row![
            heading("Reagir"),
            space().width(Length::Fill),
            icon_button(Icon::Close, text_secondary(), Some(Message::DialogClosed)),
        ]
        .align_y(Alignment::Center),
        quick,
        divider(),
        emoji_picker(section),
    ]
    .spacing(units(3))
    .width(340)
    .into()
}

fn forward_dialog<'a>(section: &'a WhatsApp, state: &'a ForwardState) -> Element<'a, Message> {
    let targets = section.forward_targets(&state.query);
    let list = targets
        .into_iter()
        .fold(Column::new().spacing(2), |col, c| {
            let chosen = state.selected.contains(&c.jid);
            let mark: Element<'_, Message> = if chosen {
                icon(Icon::Check, ICON_SIZE - 2.0, accent())
            } else {
                space().width(ICON_SIZE - 2.0).into()
            };
            col.push(
                button(
                    row![
                        avatar(section.photo_of(&c.jid), &c.title(), 30.0, WHATSAPP_GREEN),
                        text(c.title()).size(FONT_SIZE).width(Length::Fill),
                        mark,
                    ]
                    .spacing(units(3))
                    .align_y(Alignment::Center),
                )
                .padding([units(1), units(2)])
                .width(Length::Fill)
                .style(style::selectable(chosen))
                .on_press(Message::ForwardToggle(c.jid.clone())),
            )
        });
    let count = state.selected.len();
    let label = if count > 0 {
        format!("Encaminhar ({count})")
    } else {
        "Encaminhar".to_string()
    };
    column![
        heading("Encaminhar mensagem"),
        text_field("Buscar conversa", &state.query, Message::ForwardQuery),
        scrollable(container(list).padding(hub_ui_kit::iced::Padding::default().right(units(3))))
            .height(260)
            .style(style::scroll),
        row![
            space().width(Length::Fill),
            ghost_button("Cancelar", Some(Message::DialogClosed)),
            button(text(label).size(FONT_SIZE))
                .padding([units(2), units(4)])
                .style(style::primary)
                .on_press_maybe((count > 0 && !state.sending).then_some(Message::ForwardConfirm)),
        ]
        .spacing(units(2)),
    ]
    .spacing(units(3))
    .width(380)
    .into()
}

pub fn overlay<'a>(section: &'a WhatsApp, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(dialog) = &section.dialog else {
        return base;
    };
    let content: Element<'a, Message> = match dialog {
        Dialog::React(id) => react_dialog(section, id),
        Dialog::Forward(state) => forward_dialog(section, state),
        Dialog::ConfirmRevoke(id) => confirm(
            "Apagar para todos?".to_string(),
            "A mensagem some para todos na conversa. Isso não pode ser desfeito.".to_string(),
            "Apagar para todos",
            Message::RevokeConfirm(id.clone()),
            Message::DialogClosed,
        ),
        Dialog::ConfirmDeleteLocal(id) => confirm(
            "Apagar só aqui?".to_string(),
            "A mensagem sai apenas deste computador e continua no celular e para as outras pessoas."
                .to_string(),
            "Apagar só aqui",
            Message::DeleteLocalConfirm(id.clone()),
            Message::DialogClosed,
        ),
    };
    modal(base, content, Message::DialogClosed)
}
