use hub_core::platform::Os;
use hub_core::shortcuts::{Shortcut, ShortcutKind};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::widget::{
    Column, Row, button, column, container, image, mouse_area, row, scrollable, space, text,
};
use hub_ui_kit::iced::{Alignment, Color, Element, Length, Padding, mouse};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::{
    Apps, DragItem, FormMessage, Message, Modal, ShortcutForm, Shortcuts, VISIBLE_APPS, ellipsize,
};

const TILE_SIZE: f32 = 144.0;
const TILE_ICON: f32 = 44.0;
const PATH_CHARS: usize = 22;

fn kind_icon(kind: ShortcutKind) -> (Icon, Color) {
    match kind {
        ShortcutKind::Program => (Icon::App, ACCENT),
        ShortcutKind::Folder => (Icon::Folder, WARNING),
        ShortcutKind::Url => (Icon::Link, SUCCESS),
        ShortcutKind::Command => (Icon::Command, KIND_ROUTINE),
    }
}

fn symbol<'a>(kind: ShortcutKind, size: f32) -> Element<'a, Message> {
    let (glyph, color) = kind_icon(kind);
    container(icon(glyph, size * 0.55, color))
        .center_x(size)
        .center_y(size)
        .style(style::badge(color))
        .into()
}

fn picture<'a>(handle: Option<&Handle>, kind: ShortcutKind, size: f32) -> Element<'a, Message> {
    match handle {
        Some(handle) => image(handle.clone()).width(size).height(size).into(),
        None => symbol(kind, size),
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
                primary_button("Criar atalho", Some(Message::NewShortcut)),
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
                .fold(Column::new().spacing(units(3)), |col, (group, items)| {
                    col.push(group_view(section, group, items))
                }),
        )
    };
    let area = mouse_area(container(body).width(Length::Fill).height(Length::Fill))
        .on_right_press(Message::AreaMenu);
    let base = panel(content.push(area));

    match section.modal() {
        None => base.into(),
        Some(Modal::Form(form)) => modal(base, form_view(section, form), Message::ModalClosed),
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

fn group_view<'a>(
    section: &'a Shortcuts,
    group: String,
    items: Vec<(usize, &'a Shortcut)>,
) -> Element<'a, Message> {
    let collapsed = section.is_collapsed(&group);
    let dragging = section.dragging();
    let hovered = section.hovered_group() == Some(group.as_str());
    let highlight = hovered
        && match dragging {
            Some(DragItem::Group(source)) => *source != group,
            Some(DragItem::Shortcut(_)) => section.hovered_tile().is_none(),
            None => false,
        };
    let faded = matches!(dragging, Some(DragItem::Group(source)) if *source == group);
    let header = mouse_area(
        row![
            icon(
                if collapsed {
                    Icon::ChevronUp
                } else {
                    Icon::ChevronDown
                },
                ICON_SIZE - 4.0,
                TEXT_SECONDARY
            ),
            heading(group.clone()),
            small(items.len().to_string()).color(TEXT_SECONDARY),
            space().width(Length::Fill),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
    )
    .on_press(Message::GroupPressed(group.clone()))
    .interaction(if faded {
        mouse::Interaction::Grabbing
    } else {
        mouse::Interaction::Pointer
    });
    let mut content = Column::new().spacing(units(2)).push(header);
    if !collapsed {
        content = content.push(
            items
                .into_iter()
                .fold(Row::new().spacing(units(3)), |r, (index, shortcut)| {
                    r.push(tile(section, index, shortcut))
                })
                .wrap()
                .vertical_spacing(units(3)),
        );
    }
    mouse_area(
        container(content)
            .padding(units(2))
            .width(Length::Fill)
            .style(style::drop_zone(highlight, faded)),
    )
    .on_enter(Message::GroupHovered(group.clone()))
    .on_exit(Message::GroupLeft(group))
    .into()
}

fn tile<'a>(section: &'a Shortcuts, index: usize, shortcut: &'a Shortcut) -> Element<'a, Message> {
    let target = shortcut.target.for_os(section.os());
    let available = target.is_some();
    let path = target
        .map(|t| ellipsize(t, PATH_CHARS))
        .unwrap_or_else(|| "Sem alvo neste sistema".to_string());
    let content = column![
        picture(section.shortcut_icon(shortcut), shortcut.kind, TILE_ICON),
        text(shortcut.name.as_str())
            .size(FONT_SIZE)
            .color(if available {
                TEXT_PRIMARY
            } else {
                TEXT_SECONDARY
            })
            .align_x(text::Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fixed(FONT_SIZE * 2.6)),
        text(path)
            .size(FONT_SIZE_SMALL)
            .color(TEXT_SECONDARY)
            .align_x(text::Alignment::Center)
            .width(Length::Fill)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(units(2))
    .align_x(Alignment::Center);
    let dragging = section.dragging();
    let moving = dragging == Some(&DragItem::Shortcut(index));
    let target = matches!(dragging, Some(DragItem::Shortcut(_)))
        && !moving
        && section.hovered_tile() == Some(index);
    let inner = mouse_area(
        container(content)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .clip(true),
    )
    .on_press(Message::ShortcutPressed(index))
    .interaction(if moving {
        mouse::Interaction::Grabbing
    } else {
        mouse::Interaction::Pointer
    });
    let card = button(inner)
        .padding(units(3))
        .width(TILE_SIZE)
        .height(TILE_SIZE)
        .style(style::selectable(target))
        .on_press_maybe((available && !moving).then_some(Message::Open(index)));
    mouse_area(card)
        .on_right_press(Message::CardMenu(index))
        .on_enter(Message::TileHovered(index))
        .on_exit(Message::TileLeft(index))
        .into()
}

fn form_message(message: FormMessage) -> Message {
    Message::Form(message)
}

fn app_picker(section: &Shortcuts) -> Element<'_, Message> {
    let list: Element<'_, Message> = match section.apps() {
        Apps::Idle | Apps::Loading => container(muted("Procurando programas instalados"))
            .padding([units(4), 0.0])
            .center_x(Length::Fill)
            .into(),
        Apps::Failed(error) => container(body(error.as_str()).color(DANGER))
            .padding([units(4), 0.0])
            .center_x(Length::Fill)
            .into(),
        Apps::Ready(_) => {
            let items = section.visible_apps();
            if items.is_empty() {
                container(muted("Nenhum programa encontrado."))
                    .padding([units(4), 0.0])
                    .center_x(Length::Fill)
                    .into()
            } else {
                let full = items.len() == VISIBLE_APPS;
                let mut rows =
                    items
                        .into_iter()
                        .fold(Column::new().spacing(2), |col, (index, app)| {
                            let path = std::path::Path::new(&app.target);
                            col.push(
                                button(
                                    row![
                                        picture(section.icon(path), ShortcutKind::Program, 24.0),
                                        body(app.name.as_str()).width(Length::Fill),
                                    ]
                                    .spacing(units(3))
                                    .align_y(Alignment::Center),
                                )
                                .padding([units(1), units(2)])
                                .width(Length::Fill)
                                .style(style::list_item)
                                .on_press(Message::AppPicked(index)),
                            )
                        });
                if full {
                    rows = rows.push(
                        container(small("Digite para refinar a busca").color(TEXT_SECONDARY))
                            .padding([units(1), units(2)]),
                    );
                }
                scrollable(container(rows).padding(Padding::default().right(units(2))))
                    .height(200)
                    .style(style::scroll)
                    .into()
            }
        }
    };
    column![
        text_field(
            "Buscar programa instalado",
            section.app_query(),
            Message::AppQueryChanged
        ),
        container(list).padding(units(1)).style(style::elevated),
    ]
    .spacing(units(2))
    .into()
}

fn form_view<'a>(section: &'a Shortcuts, form: &'a ShortcutForm) -> Element<'a, Message> {
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
    let group_chips = section
        .known_groups()
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
        labeled("Tipo", kinds),
    ]
    .spacing(units(4));
    if form.kind == ShortcutKind::Program {
        col = col.push(labeled("Programa instalado", app_picker(section)));
    }
    col = col.push(labeled(
        "Nome",
        text_field("Ex.: VS Code", &form.name, |v| {
            form_message(FormMessage::NameChanged(v))
        }),
    ));
    let windows = text_field(hint.0, &form.windows, |v| {
        form_message(FormMessage::WindowsChanged(v))
    });
    let linux = text_field(hint.1, &form.linux, |v| {
        form_message(FormMessage::LinuxChanged(v))
    });
    col = col.push(labeled("Alvo no Windows", windows));
    col = col.push(labeled("Alvo no Linux", linux));
    if matches!(form.kind, ShortcutKind::Program | ShortcutKind::Folder) {
        let system = match section.os() {
            Os::Windows => "Windows",
            Os::Linux => "Linux",
        };
        col = col.push(
            row![
                icon_label_button(Icon::File, "Escolher arquivo", Some(Message::PickFile)),
                icon_label_button(Icon::Folder, "Escolher pasta", Some(Message::PickFolder)),
                small(format!("Preenche o alvo no {system}")).color(TEXT_SECONDARY),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
        );
    }
    col = col.push(labeled(
        "Grupo",
        column![
            text_field("Geral", &form.group, |v| {
                form_message(FormMessage::GroupChanged(v))
            })
            .on_submit(Message::FormSaved),
            group_chips,
        ]
        .spacing(units(2)),
    ));
    if let Some(error) = section.form_error() {
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
        .width(520)
        .into()
}
