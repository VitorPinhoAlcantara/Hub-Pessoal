use hub_core::agenda::recurrence::weekday_short;
use hub_core::agenda::{Priority, Reminder, Routine, Task};
use hub_core::chrono::{Datelike, NaiveDate, TimeDelta};
use hub_ui_kit::iced::widget::{
    Column, Row, button, checkbox, column, container, mouse_area, responsive, row, scrollable,
    space, text, text_editor, toggler,
};
use hub_ui_kit::iced::{Alignment, Color, Element, Length, Padding, mouse};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{Icon, style};

use crate::board::{self, Card, CardId, CardKind, DayColumn, Month};
use crate::editor::{Editor, EditorMessage};
use crate::format;
use crate::routine_form::{RecurrenceKind, RoutineFormMessage, WEEKDAYS};
use crate::{Agenda, DeleteTarget, Message, Modal, Screen};

const COLUMN_MIN_WIDTH: f32 = 220.0;
const PENDING_WIDTH: f32 = 230.0;
const CALENDAR_ITEMS: usize = 3;
const EDITOR_WIDTH: f32 = 520.0;

pub fn kind_color(kind: CardKind) -> Color {
    match kind {
        CardKind::Appointment => KIND_APPOINTMENT,
        CardKind::Routine => KIND_ROUTINE,
        CardKind::Reminder => KIND_REMINDER,
    }
}

pub fn view(agenda: &Agenda) -> Element<'_, Message> {
    let mut content = Column::new().spacing(units(3)).push(header(agenda));
    if let Some(error) = agenda.error() {
        content = content.push(error_banner(error));
    }
    if !agenda.missed().is_empty() {
        content = content.push(missed_banner(agenda.missed(), agenda.today()));
    }
    if matches!(agenda.screen(), Screen::Week | Screen::Calendar) {
        content = content.push(filters(agenda));
    }
    let body = match agenda.screen() {
        Screen::Week => week(agenda),
        Screen::Calendar => calendar(agenda),
        Screen::Routines => scroll(routines(agenda)),
        Screen::Completed => scroll(completed(agenda)),
    };
    let base = panel(content.push(body));
    match agenda.modal() {
        None => base.into(),
        Some(Modal::Editor(editor)) => modal(
            base,
            editor_view(editor, agenda.today()),
            Message::ModalClosed,
        ),
        Some(Modal::ConfirmDelete { title, .. }) => modal(
            base,
            confirm(
                "Excluir este item?".to_string(),
                format!("\"{title}\" será apagado e não dá para desfazer."),
                "Excluir",
                Message::DeleteConfirmed,
                Message::ModalClosed,
            ),
            Message::ModalClosed,
        ),
    }
}

fn header(agenda: &Agenda) -> Element<'_, Message> {
    let tab = |glyph, label, screen| {
        tab_button(
            glyph,
            label,
            agenda.screen() == screen,
            Message::ScreenSelected(screen),
        )
    };
    let tabs = row![
        tab(Icon::Columns, "Semana", Screen::Week),
        tab(Icon::Agenda, "Calendário", Screen::Calendar),
        tab(Icon::Repeat, "Rotinas", Screen::Routines),
        tab(Icon::Done, "Concluídas", Screen::Completed),
    ]
    .spacing(units(1));

    let navigation: Element<'_, Message> = match agenda.screen() {
        Screen::Week => navigator(
            Message::ShiftDays(-7),
            Message::ShiftDays(7),
            agenda.focus() != agenda.today(),
            None,
        ),
        Screen::Calendar => navigator(
            Message::MonthShifted(-1),
            Message::MonthShifted(1),
            Month::of(agenda.today()) != agenda.month(),
            Some(agenda.month().label()),
        ),
        _ => space().into(),
    };

    let create = button(
        row![
            icon(Icon::Plus, ICON_SIZE - 4.0, BG_BASE),
            text("Nova agenda").size(FONT_SIZE)
        ]
        .spacing(units(2))
        .align_y(Alignment::Center),
    )
    .padding([units(2), units(3)])
    .style(style::primary)
    .on_press(Message::OpenCreate(None));

    row![
        title("Agenda"),
        navigation,
        space().width(Length::Fill),
        tabs,
        create,
    ]
    .align_y(Alignment::Center)
    .spacing(units(4))
    .into()
}

fn navigator<'a>(
    previous: Message,
    next: Message,
    away_from_today: bool,
    label: Option<String>,
) -> Element<'a, Message> {
    let arrow = |glyph: &'static str, message: Message| {
        button(text(glyph).size(FONT_SIZE_TITLE))
            .padding([0.0, units(2) + 2.0])
            .style(style::ghost)
            .on_press(message)
    };
    let mut nav = row![arrow("‹", previous)]
        .spacing(units(1))
        .align_y(Alignment::Center);
    if let Some(label) = label {
        nav = nav.push(container(heading(label)).center_x(170));
    }
    nav = nav.push(arrow("›", next));
    nav = nav.push(ghost_button(
        "Hoje",
        away_from_today.then_some(Message::GoToday),
    ));
    nav.into()
}

fn filters(agenda: &Agenda) -> Element<'_, Message> {
    let chips = CardKind::ALL
        .into_iter()
        .fold(Row::new().spacing(units(2)), |r, kind| {
            let active = agenda.is_filtered(kind);
            let dimmed = agenda.has_filter() && !active;
            let color = kind_color(kind);
            r.push(
                button(
                    row![
                        container(space())
                            .width(10)
                            .height(10)
                            .style(style::outlined(color, dimmed)),
                        text(kind.label()).size(FONT_SIZE_SMALL).color(if dimmed {
                            TEXT_SECONDARY
                        } else {
                            TEXT_PRIMARY
                        }),
                    ]
                    .spacing(units(2))
                    .align_y(Alignment::Center),
                )
                .padding([units(1), units(3)])
                .style(style::toggle(active))
                .on_press(Message::FilterToggled(kind)),
            )
        });
    chips.into()
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

fn card_view<'a>(agenda: &'a Agenda, card: &'a Card, show_date: bool) -> Element<'a, Message> {
    let today = agenda.today();
    let color = kind_color(card.kind);
    let dragging = agenda.drag().is_some_and(|d| d.active && d.card == card.id);
    let faded = card.done || card.projected || dragging;
    let leading: Element<'_, Message> = match card.id {
        CardId::Task(id) => checkbox(card.done)
            .on_toggle(move |done| Message::TaskToggled(id, done))
            .size(15)
            .style(style::check)
            .into(),
        CardId::Reminder(_) => icon(
            Icon::Bell,
            ICON_SIZE - 3.0,
            if card.done { alpha(color, 0.5) } else { color },
        ),
        CardId::Projection { .. } => icon(Icon::Repeat, ICON_SIZE - 3.0, alpha(color, 0.6)),
    };
    let mut meta = Row::new().spacing(units(2)).align_y(Alignment::Center);
    if show_date && let Some(date) = card.date {
        meta = meta.push(small(format::short_date(date)));
    }
    if let Some(time) = card.time {
        meta = meta.push(small(time.format("%H:%M").to_string()).color(TEXT_PRIMARY));
    }
    meta = meta.push(
        small(if card.projected {
            "Rotina prevista"
        } else {
            card.kind.label()
        })
        .color(color),
    );
    if card.is_overdue(today) {
        meta = meta.push(small("atrasada").color(DANGER));
    }
    if card.high_priority {
        meta = meta.push(icon(Icon::Flag, ICON_SIZE - 7.0, DANGER));
    }
    let title_color = if faded { TEXT_SECONDARY } else { TEXT_PRIMARY };
    let content = container(
        row![
            leading,
            column![body(card.title.as_str()).color(title_color), meta]
                .spacing(units(1))
                .width(Length::Fill),
        ]
        .spacing(units(2))
        .align_y(Alignment::Start),
    )
    .padding([units(2), units(2) + 2.0])
    .width(Length::Fill)
    .style(style::outlined(color, faded));
    mouse_area(content)
        .on_press(Message::CardPressed(card.id))
        .on_right_press(Message::CardMenu(card.id))
        .interaction(if card.can_move() {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::Pointer
        })
        .into()
}

fn cards_list<'a>(
    agenda: &'a Agenda,
    cards: &'a [Card],
    show_date: bool,
    empty: &'a str,
) -> Element<'a, Message> {
    let visible: Vec<&Card> = cards.iter().filter(|c| agenda.shows(c.kind)).collect();
    let content: Element<'a, Message> = if visible.is_empty() {
        container(small(empty))
            .padding([units(4), 0.0])
            .center_x(Length::Fill)
            .into()
    } else {
        visible
            .into_iter()
            .fold(Column::new().spacing(units(2)), |col, card| {
                col.push(card_view(agenda, card, show_date))
            })
            .into()
    };
    scrollable(container(content).padding(Padding::default().right(units(2))))
        .height(Length::Fill)
        .style(style::scroll)
        .into()
}

fn day_column<'a>(agenda: &'a Agenda, day: &'a DayColumn, width: f32) -> Element<'a, Message> {
    let today = agenda.today();
    let is_today = day.date == today;
    let is_past = day.date < today;
    let drop_target =
        agenda.drag().is_some_and(|d| d.active) && agenda.hovered_day() == Some(day.date);
    let name_color = if is_today {
        ACCENT
    } else if is_past {
        TEXT_SECONDARY
    } else {
        TEXT_PRIMARY
    };
    let mut date_line = row![small(day.date.format("%d/%m").to_string())]
        .spacing(units(2))
        .align_y(Alignment::Center);
    if is_today {
        date_line = date_line.push(
            container(text("Hoje").size(FONT_SIZE_SMALL))
                .padding([1.0, units(2)])
                .style(style::pill(ACCENT)),
        );
    }
    let head = row![
        column![
            heading(board::weekday_name(day.date.weekday())).color(name_color),
            date_line,
        ]
        .spacing(2)
        .width(Length::Fill),
        icon_button(
            Icon::Plus,
            TEXT_SECONDARY,
            Some(Message::OpenCreate(Some(day.date)))
        ),
    ]
    .align_y(Alignment::Center);
    let column =
        container(column![head, cards_list(agenda, &day.cards, false, "Livre")].spacing(units(3)))
            .padding(units(3))
            .width(width)
            .height(Length::Fill)
            .style(style::day_column(
                is_today || drop_target,
                is_past && !drop_target,
            ));
    mouse_area(column)
        .on_enter(Message::DayHovered(day.date))
        .on_exit(Message::DayLeft(day.date))
        .on_right_press(Message::DayMenu(Some(day.date)))
        .into()
}

fn pending_column(agenda: &Agenda) -> Element<'_, Message> {
    let head = row![
        heading("Pendências").width(Length::Fill),
        container(text(agenda.pending().len().to_string()).size(FONT_SIZE_SMALL))
            .padding([1.0, units(2)])
            .style(style::pill(DANGER)),
    ]
    .align_y(Alignment::Center);
    container(
        column![
            head,
            small("Atrasadas há mais tempo e sem data"),
            cards_list(agenda, agenda.pending(), true, "Nada pendente"),
        ]
        .spacing(units(2)),
    )
    .padding(units(3))
    .width(PENDING_WIDTH)
    .height(Length::Fill)
    .style(style::day_column(false, false))
    .into()
}

fn week(agenda: &Agenda) -> Element<'_, Message> {
    let focus = agenda.focus();
    let board = responsive(move |size| {
        let layout = board::layout(size.width, COLUMN_MIN_WIDTH, GAP_PANEL);
        let first = focus - TimeDelta::days(layout.past as i64);
        let columns = agenda
            .days()
            .iter()
            .filter(|day| day.date >= first)
            .fold(Row::new().spacing(GAP_PANEL), |r, day| {
                r.push(day_column(agenda, day, layout.column_width))
            })
            .height(Length::Fill);
        scrollable(container(columns).padding(Padding::default().bottom(units(3))))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(6).scroller_width(6),
            ))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::scroll)
            .into()
    });
    if agenda.pending().is_empty() {
        return board.into();
    }
    row![pending_column(agenda), board]
        .spacing(GAP_PANEL)
        .height(Length::Fill)
        .into()
}

fn calendar(agenda: &Agenda) -> Element<'_, Message> {
    let today = agenda.today();
    let month = agenda.month();
    let names = WEEKDAYS
        .iter()
        .fold(Row::new().spacing(units(1)), |r, day| {
            r.push(
                container(small(weekday_short(*day)))
                    .width(Length::Fill)
                    .padding([0.0, units(2)]),
            )
        });
    let weeks = agenda
        .calendar()
        .chunks(7)
        .fold(Column::new().spacing(units(1)), |col, week| {
            col.push(
                week.iter()
                    .fold(Row::new().spacing(units(1)), |r, day| {
                        r.push(calendar_cell(
                            agenda,
                            day,
                            today,
                            day.date.month() == month.month,
                        ))
                    })
                    .height(Length::Fill),
            )
        })
        .height(Length::Fill);
    column![names, weeks]
        .spacing(units(2))
        .height(Length::Fill)
        .into()
}

fn calendar_cell<'a>(
    agenda: &'a Agenda,
    day: &'a DayColumn,
    today: NaiveDate,
    in_month: bool,
) -> Element<'a, Message> {
    let is_today = day.date == today;
    let number = text(day.date.day().to_string()).size(FONT_SIZE);
    let number: Element<'_, Message> = if is_today {
        container(number)
            .padding([1.0, units(2)])
            .style(style::pill(ACCENT))
            .into()
    } else {
        number
            .color(if in_month {
                TEXT_PRIMARY
            } else {
                TEXT_SECONDARY
            })
            .into()
    };
    let visible: Vec<&Card> = day.cards.iter().filter(|c| agenda.shows(c.kind)).collect();
    let mut items = Column::new().spacing(3).push(number);
    for card in visible.iter().take(CALENDAR_ITEMS) {
        let label = match card.time {
            Some(time) => format!("{} {}", time.format("%H:%M"), card.title),
            None => card.title.clone(),
        };
        items = items.push(
            row![
                container(space())
                    .width(6)
                    .height(6)
                    .style(style::dot(kind_color(card.kind))),
                text(label)
                    .size(FONT_SIZE_SMALL)
                    .color(if card.done {
                        TEXT_SECONDARY
                    } else {
                        TEXT_PRIMARY
                    })
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(units(1))
            .align_y(Alignment::Center),
        );
    }
    if visible.len() > CALENDAR_ITEMS {
        items = items.push(small(format!("+{} mais", visible.len() - CALENDAR_ITEMS)));
    }
    mouse_area(
        button(container(items).clip(true))
            .padding(units(2))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::calendar_cell(is_today, in_month))
            .on_press(Message::DayOpened(day.date)),
    )
    .on_right_press(Message::DayMenu(Some(day.date)))
    .into()
}

fn field<'a>(label: &'a str, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![small(label), content.into()]
        .spacing(units(1))
        .into()
}

fn choice<'a>(label: &'a str, active: bool, message: Message) -> Element<'a, Message> {
    button(text(label).size(FONT_SIZE))
        .padding([units(1) + 2.0, units(3)])
        .style(style::toggle(active))
        .on_press(message)
        .into()
}

fn editor_message(message: EditorMessage) -> Message {
    Message::Editor(message)
}

fn date_picker(editor: &Editor, month: Month, today: NaiveDate) -> Element<'_, Message> {
    let nav = row![
        button(text("‹").size(FONT_SIZE_HEADING))
            .padding([0.0, units(2)])
            .style(style::ghost)
            .on_press(editor_message(EditorMessage::PickerShifted(-1))),
        container(body(month.label())).center_x(Length::Fill),
        button(text("›").size(FONT_SIZE_HEADING))
            .padding([0.0, units(2)])
            .style(style::ghost)
            .on_press(editor_message(EditorMessage::PickerShifted(1))),
    ]
    .align_y(Alignment::Center);
    let names = WEEKDAYS.iter().fold(Row::new().spacing(2), |r, day| {
        r.push(container(small(weekday_short(*day))).center_x(Length::Fill))
    });
    let weeks = month
        .grid()
        .chunks(7)
        .fold(Column::new().spacing(2), |col, week| {
            col.push(week.iter().fold(Row::new().spacing(2), |r, &date| {
                let selected = editor.date == Some(date);
                let in_month = date.month() == month.month;
                let label = text(date.day().to_string()).size(FONT_SIZE_SMALL);
                r.push(
                    button(container(label).center_x(Length::Fill))
                        .padding([units(1), 0.0])
                        .width(Length::Fill)
                        .style(move |theme, status| {
                            if selected {
                                style::primary(theme, status)
                            } else {
                                style::calendar_cell(date == today, in_month)(theme, status)
                            }
                        })
                        .on_press(editor_message(EditorMessage::DatePicked(Some(date)))),
                )
            }))
        });
    container(column![nav, names, weeks].spacing(units(2)))
        .padding(units(3))
        .style(style::elevated)
        .into()
}

fn editor_view(editor: &Editor, today: NaiveDate) -> Element<'_, Message> {
    let heading_text = match (editor.is_new(), editor.kind) {
        (true, _) => "Nova agenda".to_string(),
        (false, kind) => format!("Editar {}", kind.label().to_lowercase()),
    };
    let mut form = Column::new().spacing(units(4)).push(
        row![
            container(space())
                .width(4)
                .height(20)
                .style(style::dot(kind_color(editor.kind))),
            title(heading_text),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    );

    if editor.is_new() {
        let kinds = CardKind::ALL
            .into_iter()
            .fold(Row::new().spacing(units(1)), |r, kind| {
                r.push(
                    button(
                        row![
                            container(space())
                                .width(8)
                                .height(8)
                                .style(style::dot(kind_color(kind))),
                            text(kind.label()).size(FONT_SIZE),
                        ]
                        .spacing(units(2))
                        .align_y(Alignment::Center),
                    )
                    .padding([units(2), units(3)])
                    .style(style::toggle(editor.kind == kind))
                    .on_press(editor_message(EditorMessage::KindSelected(kind))),
                )
            });
        form = form.push(field("Tipo", kinds));
    }

    form = form.push(field(
        "Título",
        text_field("Do que se trata?", &editor.title, |v| {
            editor_message(EditorMessage::TitleChanged(v))
        })
        .on_submit(Message::EditorSaved),
    ));

    if editor.creates_routine() {
        let routine = &editor.routine;
        let kinds = RecurrenceKind::ALL
            .iter()
            .fold(Row::new().spacing(units(1)), |r, kind| {
                r.push(choice(
                    kind.label(),
                    routine.kind == *kind,
                    editor_message(EditorMessage::Routine(RoutineFormMessage::KindSelected(
                        *kind,
                    ))),
                ))
            });
        let detail: Element<'_, Message> = match routine.kind {
            RecurrenceKind::Daily => space().height(0).into(),
            RecurrenceKind::Weekdays => WEEKDAYS
                .iter()
                .enumerate()
                .fold(Row::new().spacing(units(1)), |r, (i, day)| {
                    r.push(choice(
                        weekday_short(*day),
                        routine.weekdays[i],
                        editor_message(EditorMessage::Routine(RoutineFormMessage::WeekdayToggled(
                            i,
                        ))),
                    ))
                })
                .into(),
            RecurrenceKind::MonthDay | RecurrenceKind::EveryNDays => {
                let label = if routine.kind == RecurrenceKind::MonthDay {
                    "Dia do mês (1 a 31)"
                } else {
                    "A cada quantos dias"
                };
                text_field(label, &routine.number, |v| {
                    editor_message(EditorMessage::Routine(RoutineFormMessage::NumberChanged(v)))
                })
                .width(200)
                .into()
            }
        };
        form = form.push(field("Repetição", column![kinds, detail].spacing(units(2))));
    } else {
        let date_label = editor
            .date
            .map(|d| format!("{}/{}", format::short_date(d), d.year()))
            .unwrap_or_else(|| "Sem data".to_string());
        let tomorrow = today + TimeDelta::days(1);
        let mut chips = row![
            button(
                row![
                    icon(Icon::Agenda, ICON_SIZE - 4.0, TEXT_SECONDARY),
                    text(date_label).size(FONT_SIZE)
                ]
                .spacing(units(2))
                .align_y(Alignment::Center),
            )
            .padding([units(2), units(3)])
            .style(style::toggle(editor.picker.is_some()))
            .on_press(editor_message(EditorMessage::PickerToggled)),
            choice(
                "Hoje",
                editor.date == Some(today),
                editor_message(EditorMessage::DatePicked(Some(today)))
            ),
            choice(
                "Amanhã",
                editor.date == Some(tomorrow),
                editor_message(EditorMessage::DatePicked(Some(tomorrow)))
            ),
        ]
        .spacing(units(1))
        .align_y(Alignment::Center);
        if editor.kind != CardKind::Reminder {
            chips = chips.push(choice(
                "Sem data",
                editor.date.is_none(),
                editor_message(EditorMessage::DatePicked(None)),
            ));
        }
        let mut date_block = Column::new().spacing(units(2)).push(chips);
        if let Some(month) = editor.picker {
            date_block = date_block.push(date_picker(editor, month, today));
        }
        form = form.push(field("Data", date_block));
    }

    let time_hint = "--:--";
    let mut time_row = row![field(
        if editor.kind == CardKind::Reminder {
            "Hora"
        } else {
            "Hora, opcional"
        },
        text_field(time_hint, &editor.time, |v| {
            editor_message(EditorMessage::TimeChanged(v))
        })
        .on_submit(Message::EditorSaved)
        .width(140),
    )]
    .spacing(units(6));
    if editor.kind != CardKind::Reminder && !editor.creates_routine() {
        let priorities = [
            (Priority::Low, "Baixa"),
            (Priority::Normal, "Normal"),
            (Priority::High, "Alta"),
        ]
        .into_iter()
        .fold(Row::new().spacing(units(1)), |r, (priority, label)| {
            r.push(choice(
                label,
                editor.priority == priority,
                editor_message(EditorMessage::PrioritySelected(priority)),
            ))
        });
        time_row = time_row.push(field("Prioridade", priorities));
    }
    form = form.push(time_row);

    form = form.push(field(
        "Notas",
        text_editor(&editor.notes)
            .placeholder("Comentários, detalhes, links, o que precisar")
            .on_action(|action| editor_message(EditorMessage::NotesEdited(action)))
            .height(150)
            .padding(units(3))
            .size(FONT_SIZE)
            .style(style::editor),
    ));

    if let Some(error) = &editor.error {
        form = form.push(body(error.as_str()).color(DANGER));
    }

    let mut footer = Row::new().spacing(units(2)).align_y(Alignment::Center);
    let delete_target = match editor.target {
        crate::EditorTarget::Task(id) => Some(CardId::Task(id)),
        crate::EditorTarget::Reminder(id) => Some(CardId::Reminder(id)),
        crate::EditorTarget::New => None,
    };
    if let Some(id) = delete_target {
        footer = footer.push(
            button(
                row![
                    icon(Icon::Trash, ICON_SIZE - 4.0, DANGER),
                    text("Excluir").size(FONT_SIZE).color(DANGER)
                ]
                .spacing(units(2))
                .align_y(Alignment::Center),
            )
            .padding([units(2), units(3)])
            .style(style::danger)
            .on_press(Message::AskDelete(
                DeleteTarget::Card(id),
                editor.title.clone(),
            )),
        );
    }
    footer = footer
        .push(space().width(Length::Fill))
        .push(ghost_button("Cancelar", Some(Message::ModalClosed)))
        .push(primary_button(
            if editor.is_new() { "Criar" } else { "Salvar" },
            Some(Message::EditorSaved),
        ));
    form = form.push(footer);

    scrollable(container(form).padding(Padding::default().right(units(2))))
        .style(style::scroll)
        .width(EDITOR_WIDTH)
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
                .on_press(Message::AskDelete(
                    DeleteTarget::Routine(id),
                    routine.title.clone()
                )),
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding([units(2), units(3)])
    .style(style::outlined(KIND_ROUTINE, !routine.active))
    .into()
}

fn empty(message: &str) -> Element<'_, Message> {
    container(muted(message))
        .padding([units(8), 0.0])
        .center_x(Length::Fill)
        .into()
}

fn routines(agenda: &Agenda) -> Element<'_, Message> {
    let mut col = Column::new().spacing(units(2)).push(row![
        space().width(Length::Fill),
        primary_button("Nova rotina", Some(Message::OpenCreateRoutine)),
    ]);
    if agenda.routines().is_empty() {
        col = col.push(empty(
            "Nenhuma rotina. Rotinas criam o card do dia automaticamente.",
        ));
    } else {
        for routine in agenda.routines() {
            col = col.push(routine_row(routine));
        }
    }
    col.into()
}

fn completed_row(task: &Task, today: NaiveDate) -> Element<'_, Message> {
    let kind = Card::from_task(task).kind;
    container(
        row![
            icon(Icon::Check, ICON_SIZE - 4.0, SUCCESS),
            column![
                body(task.title.as_str()).color(TEXT_SECONDARY),
                row![
                    small(kind.label()).color(kind_color(kind)),
                    small(format::completed_meta(task, today)),
                ]
                .spacing(units(2)),
            ]
            .spacing(2)
            .width(Length::Fill),
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
        ]
        .spacing(units(3))
        .align_y(Alignment::Center),
    )
    .padding([units(2), units(3)])
    .style(style::outlined(kind_color(kind), true))
    .into()
}

fn completed(agenda: &Agenda) -> Element<'_, Message> {
    if agenda.completed().is_empty() {
        return empty("Nenhuma tarefa concluída ainda.");
    }
    agenda
        .completed()
        .iter()
        .fold(Column::new().spacing(units(2)), |col, task| {
            col.push(completed_row(task, agenda.today()))
        })
        .into()
}
