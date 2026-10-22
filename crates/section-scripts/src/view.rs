use hub_core::agenda::recurrence::weekday_short;
use hub_core::chrono::{Datelike, NaiveDateTime};
use hub_core::scripts::runner::{OutputStream, Status};
use hub_core::scripts::{self, Execution, Script};
use hub_ui_kit::iced::widget::{
    Column, Row, button, checkbox, column, container, row, scrollable, space, text, toggler,
};
use hub_ui_kit::iced::{Alignment, Color, Element, Font, Length, Padding};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::form::{FormMessage, PRESETS, ScriptForm};
use crate::{Message, Modal, Run, Scripts};

const DETAIL_WIDTH: f32 = 420.0;

fn moment(at: NaiveDateTime) -> String {
    format!(
        "{} {} {}",
        weekday_short(at.weekday()),
        at.format("%d/%m"),
        at.format("%H:%M")
    )
}

fn status_color(status: &Status) -> Color {
    match status {
        Status::Completed(0) => SUCCESS,
        Status::Cancelled => TEXT_SECONDARY,
        _ => DANGER,
    }
}

fn execution_label(execution: &Execution) -> (String, Color) {
    match (execution.finished_at, execution.exit_code) {
        (None, _) => ("Em andamento".to_string(), ACCENT),
        (Some(_), Some(0)) => ("Sucesso".to_string(), SUCCESS),
        (Some(_), Some(code)) => (format!("Código {code}"), DANGER),
        (Some(_), None) => ("Interrompido".to_string(), WARNING),
    }
}

pub fn view(section: &Scripts) -> Element<'_, Message> {
    let header = row![
        title("Scripts"),
        space().width(Length::Fill),
        icon_label_button(Icon::Folder, "Abrir pasta", Some(Message::OpenFolder)),
        primary_icon_button(Icon::Plus, "Novo script", Some(Message::NewScript)),
    ]
    .spacing(units(2))
    .align_y(Alignment::Center);

    let mut content = Column::new().spacing(units(3)).push(header);
    if let Some(error) = section.error() {
        content = content.push(
            container(
                row![
                    body(error).color(DANGER).width(Length::Fill),
                    icon_button(Icon::Close, TEXT_SECONDARY, Some(Message::ErrorDismissed)),
                ]
                .align_y(Alignment::Center),
            )
            .padding([units(1), units(3)])
            .style(style::elevated),
        );
    }

    let list: Element<'_, Message> = if section.scripts().is_empty() {
        container(
            column![
                icon(Icon::Scripts, 32.0, TEXT_SECONDARY),
                muted("Nenhum script ainda."),
                primary_button("Criar o primeiro", Some(Message::NewScript)),
            ]
            .spacing(units(3))
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding([units(10), 0.0])
        .into()
    } else {
        scroll(
            section
                .scripts()
                .iter()
                .fold(Column::new().spacing(units(2)), |col, script| {
                    col.push(script_card(section, script))
                }),
        )
    };

    let body = row![
        container(list).width(Length::Fill).height(Length::Fill),
        detail(section)
    ]
    .spacing(GAP_PANEL)
    .height(Length::Fill);
    let base = panel(content.push(body));

    match section.modal() {
        None => base.into(),
        Some(Modal::Form(form)) => modal(
            base,
            form_view(form, section.form_error(), section.now()),
            Message::ModalClosed,
        ),
        Some(Modal::ConfirmRun(id)) => {
            let name = section
                .script(id)
                .map(|s| s.meta.name.clone())
                .unwrap_or_default();
            modal(
                base,
                column![
                    heading(format!("Executar {name}?")),
                    muted("Este script pede confirmação antes de rodar."),
                    row![
                        space().width(Length::Fill),
                        ghost_button("Cancelar", Some(Message::ModalClosed)),
                        primary_button("Executar", Some(Message::RunConfirmed)),
                    ]
                    .spacing(units(2)),
                ]
                .spacing(units(4))
                .width(380),
                Message::ModalClosed,
            )
        }
        Some(Modal::ConfirmDelete(id)) => {
            let name = section
                .script(id)
                .map(|s| s.meta.name.clone())
                .unwrap_or_default();
            modal(
                base,
                confirm(
                    format!("Excluir {name}?"),
                    "Os metadados e os arquivos do script serão apagados.".to_string(),
                    "Excluir",
                    Message::DeleteConfirmed,
                    Message::ModalClosed,
                ),
                Message::ModalClosed,
            )
        }
    }
}

fn script_card<'a>(section: &'a Scripts, script: &'a Script) -> Element<'a, Message> {
    let id = script.id.clone();
    let selected = section.selected().is_some_and(|s| s.id == script.id);
    let running = section.run(&script.id).is_some_and(Run::is_running);

    let mut badges = Row::new().spacing(units(2)).align_y(Alignment::Center);
    if !script.meta.active {
        badges = badges.push(badge("Inativo", TEXT_SECONDARY));
    }
    if let Some(schedule) = &script.meta.schedule {
        let label = match scripts::next_run(schedule, section.now()) {
            Some(at) if script.meta.active => format!("Próxima: {}", moment(at)),
            _ => "Agendado".to_string(),
        };
        badges = badges.push(badge(label, ACCENT));
    }
    if script.meta.confirm {
        badges = badges.push(badge("Confirma antes", WARNING));
    }

    let mut info = column![
        row![heading(script.meta.name.as_str()), badges]
            .spacing(units(3))
            .align_y(Alignment::Center)
    ]
    .spacing(units(1))
    .width(Length::Fill);
    if !script.meta.description.is_empty() {
        info = info.push(muted(script.meta.description.as_str()));
    }
    if let Some(problem) = &script.problem {
        info = info.push(
            row![
                icon(Icon::Warning, ICON_SIZE - 6.0, WARNING),
                small(problem.as_str()).color(WARNING)
            ]
            .spacing(units(1))
            .align_y(Alignment::Center),
        );
    } else if let Some(last) = section.last(&script.id) {
        let (label, color) = execution_label(last);
        info = info.push(
            row![
                small(format!("Última: {}", moment(last.started_at))),
                small(label).color(color),
                small(last.origin.label()),
            ]
            .spacing(units(2)),
        );
    }

    let action: Element<'_, Message> = if running {
        primary_icon_button(Icon::Stop, "Parar", Some(Message::Cancel(id.clone())))
            .style(style::danger_solid)
            .into()
    } else {
        primary_icon_button(
            Icon::Play,
            "Executar",
            script.can_run().then(|| Message::RunRequested(id.clone())),
        )
        .into()
    };

    let tools = row![
        toggler(script.meta.active)
            .on_toggle({
                let id = id.clone();
                move |on| Message::ActiveToggled(id.clone(), on)
            })
            .size(16),
        icon_button(
            Icon::Edit,
            TEXT_SECONDARY,
            Some(Message::EditScript(id.clone()))
        ),
        icon_button(
            Icon::File,
            TEXT_SECONDARY,
            Some(Message::OpenFile(id.clone()))
        ),
        icon_button(
            Icon::Trash,
            TEXT_SECONDARY,
            Some(Message::AskDelete(id.clone()))
        ),
        action,
    ]
    .spacing(units(1))
    .align_y(Alignment::Center);

    button(
        row![info, tools]
            .spacing(units(3))
            .align_y(Alignment::Center),
    )
    .padding([units(3), units(4)])
    .width(Length::Fill)
    .style(style::selectable(selected))
    .on_press(Message::Selected(id))
    .into()
}

fn console_view<'a>(lines: impl Iterator<Item = (bool, &'a str)>) -> Element<'a, Message> {
    let content = lines.fold(Column::new().spacing(1), |col, (error, line)| {
        col.push(
            text(line)
                .font(Font::MONOSPACE)
                .size(FONT_SIZE_SMALL + 1.0)
                .color(if error { DANGER } else { TEXT_PRIMARY }),
        )
    });
    container(
        scrollable(container(content).padding(units(3)))
            .anchor_bottom()
            .height(Length::Fill)
            .style(style::scroll),
    )
    .height(Length::FillPortion(3))
    .width(Length::Fill)
    .style(style::console)
    .into()
}

fn detail(section: &Scripts) -> Element<'_, Message> {
    let Some(script) = section.selected() else {
        return container(muted("Selecione um script para ver a saída e o histórico."))
            .width(DETAIL_WIDTH)
            .height(Length::Fill)
            .center_x(DETAIL_WIDTH)
            .padding(units(4))
            .style(style::day_column(false, false))
            .into();
    };
    let run = section.run(&script.id);
    let viewing_history = section.history_selected();

    let (status_text, status_color): (String, Color) = match (viewing_history, run) {
        (Some(execution), _) => {
            let (label, color) = execution_label(execution);
            (
                format!("{} em {}", label, moment(execution.started_at)),
                color,
            )
        }
        (None, Some(run)) => match &run.status {
            None => ("Executando".to_string(), ACCENT),
            Some(status) => (status.describe(), status_color(status)),
        },
        (None, None) => ("Saída".to_string(), TEXT_SECONDARY),
    };

    let console: Element<'_, Message> = match (viewing_history, run) {
        (Some(execution), _) => console_view(execution.output.lines().map(|l| (false, l))),
        (None, Some(run)) => console_view(
            run.lines
                .iter()
                .map(|l| (l.stream == OutputStream::Stderr, l.text.as_str())),
        ),
        (None, None) => console_view(std::iter::once((
            false,
            "Ainda não executado nesta sessão.",
        ))),
    };

    let history =
        section
            .history()
            .iter()
            .fold(Column::new().spacing(units(1)), |col, execution| {
                let (label, color) = execution_label(execution);
                let selected = section
                    .history_selected()
                    .is_some_and(|e| e.id == execution.id);
                col.push(
                    button(
                        row![
                            container(space())
                                .width(8)
                                .height(8)
                                .style(style::dot(color)),
                            small(moment(execution.started_at)).color(TEXT_PRIMARY),
                            small(label).color(color),
                            space().width(Length::Fill),
                            small(execution.origin.label()),
                        ]
                        .spacing(units(2))
                        .align_y(Alignment::Center),
                    )
                    .padding([units(1) + 2.0, units(2)])
                    .width(Length::Fill)
                    .style(style::toggle(selected))
                    .on_press(Message::HistorySelected(execution.id)),
                )
            });
    let history: Element<'_, Message> = if section.history().is_empty() {
        small("Sem execuções registradas.").into()
    } else {
        scrollable(container(history).padding(Padding::default().right(units(2))))
            .height(Length::Fill)
            .style(style::scroll)
            .into()
    };

    let mut header = row![
        column![
            heading(script.meta.name.as_str()),
            small(status_text).color(status_color)
        ]
        .spacing(2)
        .width(Length::Fill)
    ]
    .align_y(Alignment::Center);
    if run.is_some_and(Run::can_cancel) {
        header = header.push(
            button(text("Cancelar").size(FONT_SIZE))
                .padding([units(1) + 2.0, units(3)])
                .style(style::danger_solid)
                .on_press(Message::Cancel(script.id.clone())),
        );
    }

    container(
        column![
            header,
            console,
            small("Histórico"),
            container(history).height(Length::FillPortion(2)),
        ]
        .spacing(units(3)),
    )
    .padding(units(4))
    .width(DETAIL_WIDTH)
    .height(Length::Fill)
    .style(style::day_column(false, false))
    .into()
}

fn form_message(message: FormMessage) -> Message {
    Message::Form(message)
}

fn form_view<'a>(
    form: &'a ScriptForm,
    error: Option<&'a str>,
    now: NaiveDateTime,
) -> Element<'a, Message> {
    let mut col = Column::new()
        .spacing(units(4))
        .push(title(if form.is_new() {
            "Novo script"
        } else {
            "Editar script"
        }))
        .push(labeled(
            "Nome",
            text_field("Ex.: Limpar temporários", &form.name, |v| {
                form_message(FormMessage::NameChanged(v))
            }),
        ))
        .push(labeled(
            "Descrição",
            text_field("O que ele faz", &form.description, |v| {
                form_message(FormMessage::DescriptionChanged(v))
            }),
        ));
    if form.is_new() {
        col = col.push(
            row![
                labeled(
                    "Arquivo no Windows",
                    text_field("script.ps1", &form.windows_file, |v| {
                        form_message(FormMessage::WindowsFileChanged(v))
                    })
                ),
                labeled(
                    "Arquivo no Linux",
                    text_field("script.sh", &form.linux_file, |v| {
                        form_message(FormMessage::LinuxFileChanged(v))
                    })
                ),
            ]
            .spacing(units(3)),
        );
    }
    let presets = PRESETS
        .iter()
        .fold(Row::new().spacing(units(1)), |r, (label, cron)| {
            r.push(choice(
                label,
                form.schedule.trim() == *cron,
                form_message(FormMessage::SchedulePreset(cron)),
            ))
        });
    let preview: Element<'_, Message> = match form.preview(now) {
        Ok(Some(at)) => small(format!("Próxima execução: {}", moment(at))).into(),
        Ok(None) => small("Sem agendamento: só roda quando você mandar.").into(),
        Err(error) => small(error).color(DANGER).into(),
    };
    col = col.push(labeled(
        "Agendamento (cron com segundos, opcional)",
        column![
            text_field("0 0 3 * * *", &form.schedule, |v| {
                form_message(FormMessage::ScheduleChanged(v))
            }),
            presets,
            preview,
        ]
        .spacing(units(2)),
    ));
    col = col.push(
        row![
            labeled(
                "Tempo limite (segundos)",
                text_field("300", &form.timeout, |v| {
                    form_message(FormMessage::TimeoutChanged(v))
                })
                .width(140)
            ),
            column![
                checkbox(form.confirm)
                    .label("Pedir confirmação antes de rodar")
                    .on_toggle(|on| form_message(FormMessage::ConfirmToggled(on)))
                    .size(15)
                    .text_size(FONT_SIZE)
                    .style(style::check),
                checkbox(form.active)
                    .label("Ativo")
                    .on_toggle(|on| form_message(FormMessage::ActiveToggled(on)))
                    .size(15)
                    .text_size(FONT_SIZE)
                    .style(style::check),
            ]
            .spacing(units(2)),
        ]
        .spacing(units(6))
        .align_y(Alignment::End),
    );
    if let Some(error) = error {
        col = col.push(body(error).color(DANGER));
    }
    col = col.push(
        row![
            space().width(Length::Fill),
            ghost_button("Cancelar", Some(Message::ModalClosed)),
            primary_button(
                if form.is_new() { "Criar" } else { "Salvar" },
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
