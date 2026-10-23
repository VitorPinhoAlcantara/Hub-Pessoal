use hub_core::chrono::NaiveDateTime;
use hub_core::notes::{Note, TempReset};
use hub_core::vault::VaultItem;
use hub_ui_kit::iced::widget::{
    Column, Row, button, column, container, mouse_area, row, space, text, text_editor, text_input,
};
use hub_ui_kit::iced::{Alignment, Element, Length, mouse};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::{Editing, Message, Modal, Notes, PasswordField, PasswordForm, Tab};

const LIST_WIDTH: f32 = 280.0;
const PREVIEW_CHARS: usize = 44;

fn short_date(date: NaiveDateTime) -> String {
    date.format("%d/%m %H:%M").to_string()
}

fn format_size(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let size = size as f64;
    if size < KB {
        format!("{} B", size as u64)
    } else if size < MB {
        format!("{:.1} KB", size / KB).replace('.', ",")
    } else {
        format!("{:.1} MB", size / MB).replace('.', ",")
    }
}

fn preview(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut chars = line.chars();
    let head: String = chars.by_ref().take(PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn password_field<'a>(
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> text_input::TextInput<'a, Message> {
    text_field(placeholder, value, on_input).secure(true)
}

pub fn view(section: &Notes) -> Element<'_, Message> {
    let tabs = row![
        tab_button(
            Icon::Notes,
            "Notas",
            section.tab() == Tab::Notes,
            Message::TabSelected(Tab::Notes)
        ),
        tab_button(
            Icon::Lock,
            "Cofre",
            section.tab() == Tab::Vault,
            Message::TabSelected(Tab::Vault)
        ),
    ]
    .spacing(units(1));

    let mut content = Column::new().spacing(units(3)).push(tabs);
    if let Some(error) = section.error() {
        content = content.push(
            container(
                row![
                    icon(Icon::Warning, ICON_SIZE - 4.0, danger()),
                    body(error).color(danger()).width(Length::Fill),
                    icon_button(Icon::Close, text_secondary(), Some(Message::ErrorDismissed)),
                ]
                .spacing(units(2))
                .align_y(Alignment::Center),
            )
            .padding([units(1), units(3)])
            .style(style::elevated),
        );
    }
    content = content.push(match section.tab() {
        Tab::Notes => notes_tab(section),
        Tab::Vault => vault_tab(section),
    });
    let base = panel(content);

    match section.modal() {
        None => base.into(),
        Some(Modal::ConfirmDeleteNote(id)) => {
            let name = section
                .notes()
                .iter()
                .find(|n| n.id == *id)
                .map(Note::display_title)
                .unwrap_or_default();
            modal(
                base,
                confirm(
                    format!("Excluir {name}?"),
                    "A nota será apagada de vez.".to_string(),
                    "Excluir",
                    Message::DeleteConfirmed,
                    Message::ModalClosed,
                ),
                Message::ModalClosed,
            )
        }
        Some(Modal::ConfirmRemoveItem(id)) => {
            let name = section
                .vault_items()
                .iter()
                .find(|i| i.id == *id)
                .map(|i| i.name.clone())
                .unwrap_or_default();
            modal(
                base,
                confirm(
                    format!("Remover {name} do cofre?"),
                    "O arquivo será apagado do cofre. O original, fora dele, não é afetado."
                        .to_string(),
                    "Remover",
                    Message::RemoveConfirmed,
                    Message::ModalClosed,
                ),
                Message::ModalClosed,
            )
        }
        Some(Modal::Rename { name, .. }) => modal(base, rename_view(name), Message::ModalClosed),
        Some(Modal::ChangePassword(form)) => modal(base, password_view(form), Message::ModalClosed),
    }
}

fn notes_tab(section: &Notes) -> Element<'_, Message> {
    let search = container(
        row![
            icon(Icon::Search, ICON_SIZE - 4.0, text_secondary()),
            text_field("Buscar nota", section.query(), Message::QueryChanged)
                .style(|theme, status| {
                    let mut style = style::input(theme, status);
                    style.border.width = 0.0;
                    style.background =
                        hub_ui_kit::iced::Background::Color(hub_ui_kit::iced::Color::TRANSPARENT);
                    style
                })
                .padding([units(2), units(1)]),
        ]
        .spacing(units(1))
        .align_y(Alignment::Center),
    )
    .padding([0.0, units(3)])
    .width(Length::Fill)
    .style(style::elevated);

    let list: Element<'_, Message> = if section.notes().is_empty() {
        container(muted(if section.query().trim().is_empty() {
            "Nenhuma nota ainda."
        } else {
            "Nada encontrado."
        }))
        .center_x(Length::Fill)
        .padding([units(6), 0.0])
        .into()
    } else {
        let selected = section.editing().map(|e| e.id);
        scroll(
            section
                .notes()
                .iter()
                .fold(Column::new().spacing(2), |col, note| {
                    col.push(note_item(note, selected == Some(note.id)))
                }),
        )
    };

    let left = column![
        search,
        primary_icon_button(Icon::Plus, "Nova nota", Some(Message::NewNote)).width(Length::Fill),
        list,
    ]
    .spacing(units(2))
    .width(LIST_WIDTH)
    .height(Length::Fill);

    let right: Element<'_, Message> = match section.editing() {
        Some(editing) => editor_view(section, editing),
        None => container(
            column![
                icon(Icon::Notes, 32.0, text_secondary()),
                muted("Selecione uma nota ou crie uma nova."),
                primary_button("Nova nota", Some(Message::NewNote)),
            ]
            .spacing(units(3))
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into(),
    };

    row![
        left,
        container(right).width(Length::Fill).height(Length::Fill)
    ]
    .spacing(units(4))
    .height(Length::Fill)
    .into()
}

fn note_item(note: &Note, selected: bool) -> Element<'_, Message> {
    let mut head = Row::new().spacing(units(2)).align_y(Alignment::Center);
    if note.favorite {
        head = head.push(icon(Icon::Star, 14.0, warning()));
    }
    head = head.push(
        text(note.display_title())
            .size(FONT_SIZE)
            .color(text_primary())
            .width(Length::Fill)
            .wrapping(text::Wrapping::None),
    );
    if note.reset != TempReset::None {
        head = head.push(badge(note.reset.label(), accent()));
    }
    let lines = column![
        head,
        row![
            small(preview(&note.body))
                .width(Length::Fill)
                .wrapping(text::Wrapping::None),
            small(short_date(note.updated_at)),
        ]
        .spacing(units(2)),
    ]
    .spacing(2);
    mouse_area(
        button(lines)
            .padding([units(2), units(3)])
            .width(Length::Fill)
            .style(style::selectable(selected))
            .on_press(Message::NoteSelected(note.id)),
    )
    .on_right_press(Message::NoteMenu(note.id))
    .into()
}

fn editor_view<'a>(section: &'a Notes, editing: &'a Editing) -> Element<'a, Message> {
    let note = section.selected();
    let favorite = note.is_some_and(|n| n.favorite);
    let reset = note.map(|n| n.reset).unwrap_or_default();
    let header = row![
        text_field("Título", &editing.title, Message::TitleChanged).width(Length::Fill),
        with_tooltip(
            icon_button(
                Icon::Star,
                if favorite {
                    warning()
                } else {
                    text_secondary()
                },
                Some(Message::ToggleFavorite),
            ),
            if favorite {
                "Remover dos favoritos"
            } else {
                "Favoritar"
            },
            hub_ui_kit::iced::widget::tooltip::Position::Bottom,
        ),
        with_tooltip(
            icon_button(
                Icon::Trash,
                danger(),
                Some(Message::AskDeleteNote(editing.id)),
            ),
            "Excluir",
            hub_ui_kit::iced::widget::tooltip::Position::Bottom,
        ),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center);

    let body_editor = text_editor(&editing.content)
        .placeholder("Escreva aqui")
        .on_action(Message::BodyAction)
        .height(Length::Fill)
        .padding(units(3))
        .size(FONT_SIZE)
        .style(style::editor);

    let choices = TempReset::ALL
        .into_iter()
        .fold(Row::new().spacing(units(1)), |r, option| {
            r.push(choice(
                option.label(),
                reset == option,
                Message::ResetSelected(option),
            ))
        });
    let explanation = match reset {
        TempReset::None => "",
        TempReset::Daily => "O conteúdo é apagado todo dia à meia noite",
        TempReset::Weekly => "O conteúdo é apagado toda segunda feira",
    };
    let footer = row![
        small("Tipo"),
        choices,
        muted(explanation),
        space().width(Length::Fill),
        small(if section.is_dirty() {
            "Salvando".to_string()
        } else {
            note.map(|n| format!("Atualizada em {}", short_date(n.updated_at)))
                .unwrap_or_default()
        }),
    ]
    .spacing(units(3))
    .align_y(Alignment::Center);

    column![header, body_editor, footer]
        .spacing(units(3))
        .height(Length::Fill)
        .into()
}

fn vault_tab(section: &Notes) -> Element<'_, Message> {
    if section.vault_unlocked() {
        unlocked_view(section)
    } else if section.vault_exists() {
        locked_view(section)
    } else {
        create_view(section)
    }
}

fn centered<'a>(content: Column<'a, Message>) -> Element<'a, Message> {
    container(content.spacing(units(3)).width(380))
        .center_x(Length::Fill)
        .padding([units(8), 0.0])
        .into()
}

fn create_view(section: &Notes) -> Element<'_, Message> {
    let can = section.can_create() && section.busy().is_none();
    let mut col = column![
        row![
            icon(Icon::Lock, 24.0, accent()),
            heading("Criar cofre")
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
        muted("O cofre guarda documentos cifrados, protegidos por uma senha só sua."),
        labeled(
            "Senha",
            password_field("Senha", section.password(), Message::PasswordChanged)
        ),
        labeled(
            "Confirmar senha",
            password_field("Confirmar senha", section.confirm(), Message::ConfirmChanged)
                .on_submit_maybe(can.then_some(Message::CreateVault)),
        ),
        container(
            row![
                icon(Icon::Warning, ICON_SIZE - 2.0, warning()),
                body("Se você esquecer esta senha, não há como recuperar os documentos guardados no cofre.")
                    .color(warning())
                    .width(Length::Fill),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
        )
        .padding(units(3))
        .style(style::outlined(warning(), false)),
    ];
    if let Some(busy) = section.busy() {
        col = col.push(muted(busy));
    }
    col = col.push(primary_button(
        "Criar cofre",
        can.then_some(Message::CreateVault),
    ));
    centered(col)
}

fn locked_view(section: &Notes) -> Element<'_, Message> {
    let can = !section.password().is_empty() && section.busy().is_none();
    let mut col = column![
        row![icon(Icon::Lock, 24.0, accent()), heading("Cofre trancado")]
            .spacing(units(2))
            .align_y(Alignment::Center),
        password_field("Senha", section.password(), Message::PasswordChanged)
            .on_submit_maybe(can.then_some(Message::Unlock)),
    ];
    if section.wrong_password() {
        col = col.push(body("Senha incorreta").color(danger()));
    }
    if let Some(busy) = section.busy() {
        col = col.push(muted(busy));
    }
    col = col.push(primary_button(
        "Desbloquear",
        can.then_some(Message::Unlock),
    ));
    centered(col)
}

fn unlocked_view(section: &Notes) -> Element<'_, Message> {
    let idle = section.busy().is_none();
    let bar = row![
        icon_label_button(
            Icon::Plus,
            "Adicionar arquivos",
            idle.then_some(Message::AddFiles)
        ),
        icon_label_button(
            Icon::Edit,
            "Trocar senha",
            Some(Message::OpenChangePassword)
        ),
        space().width(Length::Fill),
        section
            .busy()
            .map(|b| Element::from(muted(b.to_string())))
            .unwrap_or_else(|| space().into()),
        icon_label_button(Icon::Lock, "Trancar", Some(Message::LockVault)),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center);

    let list: Element<'_, Message> = if section.vault_items().is_empty() {
        container(
            column![
                icon(Icon::Unlock, 32.0, text_secondary()),
                muted("O cofre está vazio."),
            ]
            .spacing(units(3))
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding([units(8), 0.0])
        .into()
    } else {
        scroll(
            section
                .vault_items()
                .iter()
                .fold(Column::new().spacing(units(1)), |col, item| {
                    col.push(vault_item(item))
                }),
        )
    };

    column![
        bar,
        small("Os arquivos originais continuam onde estavam; apague-os você mesmo se quiser mantê-los só no cofre."),
        list,
    ]
    .spacing(units(3))
    .height(Length::Fill)
    .into()
}

fn vault_item(item: &VaultItem) -> Element<'_, Message> {
    mouse_area(
        container(
            row![
                icon(Icon::File, ICON_SIZE, text_secondary()),
                body(item.name.as_str()).width(Length::Fill),
                small(format_size(item.size)),
                small(short_date(item.added_at)),
                icon_button(
                    Icon::External,
                    text_secondary(),
                    Some(Message::OpenItem(item.id.clone()))
                ),
            ]
            .spacing(units(3))
            .align_y(Alignment::Center),
        )
        .padding([units(1), units(3)])
        .width(Length::Fill)
        .style(style::elevated),
    )
    .on_double_click(Message::OpenItem(item.id.clone()))
    .on_right_press(Message::ItemMenu(item.id.clone()))
    .interaction(mouse::Interaction::Pointer)
    .into()
}

fn rename_view(name: &str) -> Element<'_, Message> {
    let valid = !name.trim().is_empty();
    column![
        title("Renomear arquivo"),
        text_field("Nome", name, Message::RenameChanged)
            .on_submit_maybe(valid.then_some(Message::RenameConfirmed)),
        row![
            space().width(Length::Fill),
            ghost_button("Cancelar", Some(Message::ModalClosed)),
            primary_button("Renomear", valid.then_some(Message::RenameConfirmed)),
        ]
        .spacing(units(2)),
    ]
    .spacing(units(4))
    .width(420)
    .into()
}

fn password_view(form: &PasswordForm) -> Element<'_, Message> {
    let field =
        |field: PasswordField| move |value| Message::PasswordFieldChanged(field.clone(), value);
    let mut col = column![
        title("Trocar senha"),
        labeled(
            "Senha atual",
            password_field("Senha atual", &form.old, field(PasswordField::Old))
        ),
        labeled(
            "Nova senha",
            password_field("Nova senha", &form.new, field(PasswordField::New))
        ),
        labeled(
            "Confirmar nova senha",
            password_field(
                "Confirmar nova senha",
                &form.confirm,
                field(PasswordField::Confirm)
            )
            .on_submit_maybe(form.can_submit().then_some(Message::PasswordSubmitted)),
        ),
    ]
    .spacing(units(3))
    .width(420);
    if let Some(error) = &form.error {
        col = col.push(body(error.as_str()).color(danger()));
    }
    if form.busy {
        col = col.push(muted("Trocando senha"));
    }
    col.push(
        row![
            space().width(Length::Fill),
            ghost_button("Cancelar", Some(Message::ModalClosed)),
            primary_button(
                "Trocar senha",
                form.can_submit().then_some(Message::PasswordSubmitted)
            ),
        ]
        .spacing(units(2)),
    )
    .into()
}
