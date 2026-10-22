use hub_core::platform::Os;
use hub_core::shortcuts::{Shortcut, ShortcutKind};
use hub_ui_kit::iced::widget::{
    Column, Row, button, column, container, row, scrollable, space, text,
};
use hub_ui_kit::iced::{Alignment, Color, Element, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::{FormMessage, Message, Modal, ShortcutForm, Shortcuts, Suggest};

const TILE_WIDTH: f32 = 190.0;

fn kind_icon(kind: ShortcutKind) -> (Icon, Color) {
    match kind {
        ShortcutKind::Program => (Icon::App, ACCENT),
        ShortcutKind::Folder => (Icon::Folder, WARNING),
        ShortcutKind::Url => (Icon::Link, SUCCESS),
        ShortcutKind::Command => (Icon::Command, KIND_ROUTINE),
    }
}

pub fn view(section: &Shortcuts) -> Element<'_, Message> {
    let search = container(
        row![
            icon(Icon::Search, ICON_SIZE - 4.0, TEXT_SECONDARY),
            text_field("Buscar atalho", section.query(), Message::QueryChanged)
                .style(|theme, status| {
                    let mut style = style::input(theme, status);
                    style.border.width = 0.0;
                    style.background = hub_ui_kit::iced::Background::Color(Color::TRANSPARENT);
                    style
                })
                .padding([units(2), units(1)]),
        ]
        .spacing(units(1))
        .align_y(Alignment::Center),
    )
    .padding([0.0, units(3)])
    .width(280)
    .style(style::elevated);

    let header = row![
        title("Atalhos"),
        space().width(Length::Fill),
        search,
        icon_label_button(Icon::App, "Sugerir programas", Some(Message::SuggestOpened)),
        primary_icon_button(Icon::Plus, "Novo atalho", Some(Message::NewShortcut)),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center);

    let mut content = Column::new().spacing(units(3)).push(header);
    for (problem, color) in [(section.problem(), WARNING), (section.error(), DANGER)] {
        if let Some(problem) = problem {
            content = content.push(
                container(
                    row![
                        icon(Icon::Warning, ICON_SIZE - 4.0, color),
                        body(problem).color(color).width(Length::Fill),
                        icon_button(Icon::Close, TEXT_SECONDARY, Some(Message::ErrorDismissed)),
                    ]
                    .spacing(units(2))
                    .align_y(Alignment::Center),
                )
                .padding([units(1), units(3)])
                .style(style::elevated),
            );
        }
    }

    let groups = section.grouped();
    let body: Element<'_, Message> = if section.shortcuts().is_empty() {
        container(
            column![
                icon(Icon::Shortcuts, 32.0, TEXT_SECONDARY),
                muted("Nenhum atalho ainda."),
                row![
                    ghost_button("Sugerir programas", Some(Message::SuggestOpened)),
                    primary_button("Criar atalho", Some(Message::NewShortcut)),
                ]
                .spacing(units(2)),
            ]
            .spacing(units(3))
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding([units(10), 0.0])
        .into()
    } else if groups.is_empty() {
        container(muted("Nada encontrado."))
            .center_x(Length::Fill)
            .padding([units(8), 0.0])
            .into()
    } else {
        scroll(
            groups
                .into_iter()
                .fold(Column::new().spacing(units(5)), |col, (group, items)| {
                    let tiles = items
                        .into_iter()
                        .fold(Row::new().spacing(units(2)), |r, (index, shortcut)| {
                            r.push(tile(section.os(), index, shortcut))
                        })
                        .wrap()
                        .vertical_spacing(units(2));
                    col.push(column![heading(group), tiles].spacing(units(2)))
                }),
        )
    };
    let base = panel(content.push(body));

    match section.modal() {
        None => base.into(),
        Some(Modal::Form(form)) => modal(
            base,
            form_view(form, section.known_groups(), section.form_error()),
            Message::ModalClosed,
        ),
        Some(Modal::Suggest(suggest)) => modal(base, suggest_view(suggest), Message::ModalClosed),
        Some(Modal::ConfirmDelete(index)) => {
            let name = section
                .shortcuts()
                .get(*index)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            modal(
                base,
                confirm(
                    format!("Excluir {name}?"),
                    "O atalho sai da lista; o programa ou pasta não é afetado.".to_string(),
                    "Excluir",
                    Message::DeleteConfirmed,
                    Message::ModalClosed,
                ),
                Message::ModalClosed,
            )
        }
    }
}

fn tile(os: Os, index: usize, shortcut: &Shortcut) -> Element<'_, Message> {
    let (glyph, color) = kind_icon(shortcut.kind);
    let target = shortcut.target.for_os(os);
    let available = target.is_some();
    let badge_icon = container(icon(glyph, ICON_SIZE, color))
        .padding(units(2))
        .style(style::badge(color));
    let tools = row![
        icon_button(Icon::Edit, TEXT_SECONDARY, Some(Message::Edit(index))),
        icon_button(Icon::Trash, TEXT_SECONDARY, Some(Message::AskDelete(index))),
    ];
    let content = column![
        row![badge_icon, space().width(Length::Fill), tools].align_y(Alignment::Start),
        body(shortcut.name.as_str()).color(if available {
            TEXT_PRIMARY
        } else {
            TEXT_SECONDARY
        }),
        text(target.unwrap_or("Sem alvo neste sistema"))
            .size(FONT_SIZE_SMALL)
            .color(TEXT_SECONDARY)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(units(2));
    button(container(content).clip(true))
        .padding(units(3))
        .width(TILE_WIDTH)
        .style(style::selectable(false))
        .on_press_maybe(available.then_some(Message::Open(index)))
        .into()
}

fn form_message(message: FormMessage) -> Message {
    Message::Form(message)
}

fn form_view<'a>(
    form: &'a ShortcutForm,
    groups: Vec<String>,
    error: Option<&'a str>,
) -> Element<'a, Message> {
    let kinds = ShortcutKind::ALL
        .into_iter()
        .fold(Row::new().spacing(units(1)), |r, kind| {
            let (glyph, color) = kind_icon(kind);
            r.push(
                button(
                    row![
                        icon(glyph, ICON_SIZE - 4.0, color),
                        text(kind.label()).size(FONT_SIZE)
                    ]
                    .spacing(units(2))
                    .align_y(Alignment::Center),
                )
                .padding([units(2), units(3)])
                .style(style::toggle(form.kind == kind))
                .on_press(form_message(FormMessage::KindSelected(kind))),
            )
        });
    let hint = match form.kind {
        ShortcutKind::Program => ("code, caminho do .exe ou .lnk", "code ou arquivo .desktop"),
        ShortcutKind::Folder => ("C:\\Projetos", "~/projetos"),
        ShortcutKind::Url => ("https://exemplo.com", "https://exemplo.com"),
        ShortcutKind::Command => ("comando do cmd", "comando do shell"),
    };
    let group_chips = groups
        .into_iter()
        .fold(Row::new().spacing(units(1)), |r, group| {
            let active = form.group == group;
            r.push(
                button(text(group.clone()).size(FONT_SIZE_SMALL))
                    .padding([units(1), units(2)])
                    .style(style::toggle(active))
                    .on_press(form_message(FormMessage::GroupChanged(group))),
            )
        })
        .wrap();
    let mut col = column![
        title(if form.editing.is_some() {
            "Editar atalho"
        } else {
            "Novo atalho"
        }),
        labeled(
            "Nome",
            text_field("Ex.: VS Code", &form.name, |v| {
                form_message(FormMessage::NameChanged(v))
            })
        ),
        labeled("Tipo", kinds),
        labeled(
            "Alvo no Windows",
            text_field(hint.0, &form.windows, |v| {
                form_message(FormMessage::WindowsChanged(v))
            })
        ),
        labeled(
            "Alvo no Linux",
            text_field(hint.1, &form.linux, |v| {
                form_message(FormMessage::LinuxChanged(v))
            })
        ),
        labeled(
            "Grupo",
            column![
                text_field("Geral", &form.group, |v| {
                    form_message(FormMessage::GroupChanged(v))
                })
                .on_submit(Message::FormSaved),
                group_chips,
            ]
            .spacing(units(2))
        ),
    ]
    .spacing(units(4));
    if let Some(error) = error {
        col = col.push(body(error).color(DANGER));
    }
    col = col.push(
        row![
            space().width(Length::Fill),
            ghost_button("Cancelar", Some(Message::ModalClosed)),
            primary_button(
                if form.editing.is_some() {
                    "Salvar"
                } else {
                    "Criar"
                },
                Some(Message::FormSaved)
            ),
        ]
        .spacing(units(2)),
    );
    scrollable(container(col).padding(Padding::default().right(units(2))))
        .style(style::scroll)
        .width(480)
        .into()
}

fn suggest_view(suggest: &Suggest) -> Element<'_, Message> {
    let list: Element<'_, Message> = match &suggest.apps {
        None => container(muted("Procurando programas instalados"))
            .padding([units(6), 0.0])
            .center_x(Length::Fill)
            .into(),
        Some(_) => {
            let items = suggest.filtered();
            if items.is_empty() {
                container(muted(
                    suggest
                        .error
                        .clone()
                        .unwrap_or_else(|| "Nenhum programa encontrado.".to_string()),
                ))
                .padding([units(6), 0.0])
                .center_x(Length::Fill)
                .into()
            } else {
                scrollable(
                    container(items.into_iter().fold(
                        Column::new().spacing(units(1)),
                        |col, (index, app)| {
                            col.push(
                                button(
                                    row![
                                        icon(Icon::App, ICON_SIZE - 4.0, ACCENT),
                                        column![
                                            body(app.name.as_str()),
                                            text(app.target.as_str())
                                                .size(FONT_SIZE_SMALL)
                                                .color(TEXT_SECONDARY)
                                                .wrapping(text::Wrapping::None),
                                        ]
                                        .spacing(2),
                                    ]
                                    .spacing(units(3))
                                    .align_y(Alignment::Center),
                                )
                                .padding([units(2), units(3)])
                                .width(Length::Fill)
                                .style(style::list_item)
                                .on_press(Message::AppPicked(index)),
                            )
                        },
                    ))
                    .padding(Padding::default().right(units(2))),
                )
                .height(380)
                .style(style::scroll)
                .into()
            }
        }
    };
    column![
        title("Sugerir programas"),
        text_field("Buscar", &suggest.query, Message::SuggestQueryChanged),
        list,
        row![
            space().width(Length::Fill),
            ghost_button("Fechar", Some(Message::ModalClosed))
        ],
    ]
    .spacing(units(4))
    .width(480)
    .into()
}
