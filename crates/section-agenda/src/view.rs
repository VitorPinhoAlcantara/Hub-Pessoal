use hub_core::agenda::{Priority, Reminder, Routine, Task};
use hub_core::chrono::NaiveDate;
use hub_ui_kit::iced::widget::{
    Column, button, checkbox, column, container, row, space, text, toggler,
};
use hub_ui_kit::iced::{Alignment, Element, Length};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::format;
use crate::routine_form::{RecurrenceKind, RoutineForm, RoutineFormMessage, WEEKDAYS};
use crate::{Agenda, Message, Screen};

pub fn view(agenda: &Agenda) -> Element<'_, Message> {
    let tabs = row![
        tab(Icon::Sun, "Hoje", Screen::Today, agenda),
        tab(Icon::List, "Próximos", Screen::Upcoming, agenda),
        tab(Icon::Repeat, "Rotinas", Screen::Routines, agenda),
        tab(Icon::Done, "Concluídas", Screen::Completed, agenda),
    ]
    .spacing(units(1));

    let header = row![title("Agenda"), space().width(Length::Fill), tabs]
        .align_y(Alignment::Center)
        .spacing(units(4));

    let mut content = Column::new().spacing(units(3)).push(header);

    if let Some(error) = agenda.error() {
        content = content.push(error_banner(error));
    }
    if !agenda.missed().is_empty() {
        content = content.push(missed_banner(agenda.missed(), agenda.current_day));
    }
    if matches!(agenda.screen(), Screen::Today | Screen::Upcoming) {
        content = content.push(
            text_field(
                "Adicionar tarefa, por exemplo: pagar conta amanhã 10h",
                agenda.quick_add_text(),
                Message::QuickAddChanged,
            )
            .on_submit(Message::QuickAddSubmitted),
        );
    }

    let body = match agenda.screen() {
        Screen::Today => today(agenda),
        Screen::Upcoming => upcoming(agenda),
        Screen::Routines => routines(agenda),
        Screen::Completed => completed(agenda),
    };
    content = content.push(scroll(body));

    panel(content).into()
}

fn tab<'a>(glyph: Icon, label: &'a str, screen: Screen, agenda: &Agenda) -> Element<'a, Message> {
    tab_button(
        glyph,
        label,
        agenda.screen() == screen,
        Message::ScreenSelected(screen),
    )
    .into()
}

fn error_banner(error: &str) -> Element<'_, Message> {
    container(
        row![
            body(error).color(DANGER).width(Length::Fill),
            icon_button(Icon::Close, TEXT_SECONDARY, Some(Message::ErrorDismissed)),
        ]
        .align_y(Alignment::Center)
        .spacing(units(2)),
    )
    .padding([units(1), units(3)])
    .style(style::elevated)
    .into()
}

fn missed_banner(missed: &[Reminder], today: NaiveDate) -> Element<'_, Message> {
    let list = missed
        .iter()
        .fold(Column::new().spacing(units(1)), |col, r| {
            col.push(
                row![
                    body(r.title.as_str()).width(Length::Fill),
                    small(format::moment(r.effective_time(), today)),
                ]
                .spacing(units(2)),
            )
        });
    let label = if missed.len() == 1 {
        "1 lembrete passou enquanto o hub estava fechado".to_string()
    } else {
        format!(
            "{} lembretes passaram enquanto o hub estava fechado",
            missed.len()
        )
    };
    container(
        column![
            row![
                icon(Icon::Bell, ICON_SIZE - 2.0, ACCENT),
                heading(label).width(Length::Fill),
                ghost_button("Dispensar", Some(Message::MissedDismissed)),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
            list,
        ]
        .spacing(units(2)),
    )
    .padding(units(3))
    .style(style::banner)
    .into()
}

fn empty(message: &str) -> Element<'_, Message> {
    container(muted(message))
        .padding([units(8), 0.0])
        .center_x(Length::Fill)
        .into()
}

fn group<'a>(
    label: String,
    color: hub_ui_kit::iced::Color,
    items: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut col = Column::new()
        .spacing(units(1))
        .push(container(small(label).color(color)).padding([0.0, units(2)]));
    for item in items {
        col = col.push(item);
    }
    col.into()
}

fn task_row<'a>(task: &'a Task, meta: String) -> Element<'a, Message> {
    let done = task.is_completed();
    let id = task.id;
    let mut info =
        row![body(task.title.as_str()).color(if done { TEXT_SECONDARY } else { TEXT_PRIMARY })]
            .spacing(units(2))
            .align_y(Alignment::Center);
    if task.routine_id.is_some() {
        info = info.push(icon(Icon::Repeat, ICON_SIZE - 6.0, TEXT_SECONDARY));
    }
    if task.priority == Priority::High {
        info = info.push(icon(Icon::Flag, ICON_SIZE - 6.0, DANGER));
    }
    let mut line = row![
        checkbox(done)
            .on_toggle(move |checked| Message::TaskToggled(id, checked))
            .size(16)
            .style(style::check),
        info.width(Length::Fill),
    ]
    .spacing(units(3))
    .align_y(Alignment::Center);
    if !meta.is_empty() {
        line = line.push(small(meta));
    }
    line = line.push(
        button(icon(Icon::Trash, ICON_SIZE - 4.0, TEXT_SECONDARY))
            .padding(units(1))
            .style(style::danger)
            .on_press(Message::TaskDeleted(id)),
    );
    container(line)
        .padding([units(1) + 2.0, units(2)])
        .width(Length::Fill)
        .into()
}

fn today(agenda: &Agenda) -> Element<'_, Message> {
    let view = agenda.today();
    let day = agenda.current_day;
    if view.overdue.is_empty() && view.today.is_empty() && view.undated.is_empty() {
        return empty("Nada para hoje. Use o campo acima para adicionar uma tarefa.");
    }
    let mut col = Column::new().spacing(units(4));
    if !view.overdue.is_empty() {
        col = col.push(group(
            "Atrasadas".into(),
            DANGER,
            view.overdue
                .iter()
                .map(|t| task_row(t, format::task_meta(t, day, true)))
                .collect(),
        ));
    }
    if !view.today.is_empty() {
        col = col.push(group(
            format::day_label(day, day),
            TEXT_SECONDARY,
            view.today
                .iter()
                .map(|t| task_row(t, format::task_meta(t, day, false)))
                .collect(),
        ));
    }
    if !view.undated.is_empty() {
        col = col.push(group(
            "Sem data".into(),
            TEXT_SECONDARY,
            view.undated
                .iter()
                .map(|t| task_row(t, String::new()))
                .collect(),
        ));
    }
    col.into()
}

fn upcoming(agenda: &Agenda) -> Element<'_, Message> {
    let day = agenda.current_day;
    if agenda.upcoming().is_empty() {
        return empty("Nenhuma tarefa nos próximos 14 dias.");
    }
    agenda
        .upcoming()
        .iter()
        .fold(Column::new().spacing(units(4)), |col, (date, tasks)| {
            let color = if format::is_tomorrow(*date, day) {
                ACCENT
            } else {
                TEXT_SECONDARY
            };
            col.push(group(
                format::day_label(*date, day),
                color,
                tasks
                    .iter()
                    .map(|t| task_row(t, format::task_meta(t, day, false)))
                    .collect(),
            ))
        })
        .into()
}

fn routine_row(routine: &Routine) -> Element<'_, Message> {
    let id = routine.id;
    container(
        row![
            toggler(routine.active)
                .on_toggle(move |on| Message::RoutineToggled(id, on))
                .size(16),
            column![
                body(routine.title.as_str()).color(if routine.active {
                    TEXT_PRIMARY
                } else {
                    TEXT_SECONDARY
                }),
                small(format::routine_meta(routine)),
            ]
            .spacing(2)
            .width(Length::Fill),
            button(icon(Icon::Trash, ICON_SIZE - 4.0, TEXT_SECONDARY))
                .padding(units(1))
                .style(style::danger)
                .on_press(Message::RoutineDeleted(id)),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding([units(2), units(2)])
    .into()
}

fn routine_form(form: &RoutineForm) -> Element<'_, Message> {
    let kinds = RecurrenceKind::ALL
        .iter()
        .fold(row![].spacing(units(1)), |r, kind| {
            r.push(
                button(text(kind.label()).size(FONT_SIZE))
                    .padding([units(1) + 2.0, units(3)])
                    .style(style::toggle(form.kind == *kind))
                    .on_press(Message::RoutineForm(RoutineFormMessage::KindSelected(
                        *kind,
                    ))),
            )
        });
    let detail: Element<'_, Message> = match form.kind {
        RecurrenceKind::Daily => space().height(0).into(),
        RecurrenceKind::Weekdays => WEEKDAYS
            .iter()
            .enumerate()
            .fold(row![].spacing(units(1)), |r, (i, day)| {
                r.push(
                    button(text(hub_core::agenda::recurrence::weekday_short(*day)).size(FONT_SIZE))
                        .padding([units(1) + 2.0, units(3)])
                        .style(style::toggle(form.weekdays[i]))
                        .on_press(Message::RoutineForm(RoutineFormMessage::WeekdayToggled(i))),
                )
            })
            .into(),
        RecurrenceKind::MonthDay | RecurrenceKind::EveryNDays => {
            let label = if form.kind == RecurrenceKind::MonthDay {
                "Dia do mês (1 a 31)"
            } else {
                "A cada quantos dias"
            };
            text_field(label, &form.number, |v| {
                Message::RoutineForm(RoutineFormMessage::NumberChanged(v))
            })
            .width(200)
            .into()
        }
    };
    container(
        column![
            heading("Nova rotina"),
            text_field("Nome da rotina", &form.title, |v| {
                Message::RoutineForm(RoutineFormMessage::TitleChanged(v))
            })
            .on_submit(Message::RoutineSubmitted),
            kinds,
            detail,
            row![
                text_field("Hora (opcional), ex. 08:00", &form.time, |v| {
                    Message::RoutineForm(RoutineFormMessage::TimeChanged(v))
                })
                .on_submit(Message::RoutineSubmitted)
                .width(220),
                space().width(Length::Fill),
                ghost_button("Cancelar", Some(Message::RoutineFormClosed)),
                primary_button("Salvar rotina", Some(Message::RoutineSubmitted)),
            ]
            .spacing(units(2))
            .align_y(Alignment::Center),
        ]
        .spacing(units(3)),
    )
    .padding(units(4))
    .style(style::elevated)
    .into()
}

fn routines(agenda: &Agenda) -> Element<'_, Message> {
    let mut col = Column::new().spacing(units(2));
    match agenda.routine_form() {
        Some(form) => col = col.push(routine_form(form)),
        None => {
            col = col.push(row![
                space().width(Length::Fill),
                primary_button("Nova rotina", Some(Message::RoutineFormOpened)),
            ])
        }
    }
    if agenda.routines().is_empty() {
        col = col.push(empty(
            "Nenhuma rotina. Rotinas criam a tarefa do dia automaticamente.",
        ));
    } else {
        for routine in agenda.routines() {
            col = col.push(routine_row(routine));
        }
    }
    col.into()
}

fn completed(agenda: &Agenda) -> Element<'_, Message> {
    if agenda.completed().is_empty() {
        return empty("Nenhuma tarefa concluída ainda.");
    }
    let day = agenda.current_day;
    agenda
        .completed()
        .iter()
        .fold(Column::new().spacing(units(1)), |col, task| {
            col.push(
                row![task_row(task, format::completed_meta(task, day)),]
                    .push(
                        button(
                            row![
                                icon(Icon::Undo, ICON_SIZE - 6.0, TEXT_SECONDARY),
                                small("Desfazer")
                            ]
                            .spacing(units(1))
                            .align_y(Alignment::Center),
                        )
                        .padding([units(1), units(2)])
                        .style(style::ghost)
                        .on_press(Message::TaskToggled(task.id, false)),
                    )
                    .align_y(Alignment::Center),
            )
        })
        .into()
}
