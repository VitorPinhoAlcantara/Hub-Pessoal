use std::sync::Arc;

use hub_core::agenda::{self, Priority, Recurrence};
use hub_core::chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::{Config, Database, HubContext, HubPaths};
use hub_ui_kit::iced::keyboard::Modifiers;
use hub_ui_kit::iced::widget::text_editor;

use super::*;
use crate::board::CardKind;

fn setup() -> (Agenda, HubContext) {
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 14:00")),
    );
    (Agenda::new(ctx.clone(), Vec::new()), ctx)
}

fn d(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn send(agenda: &mut Agenda, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = agenda.update(message);
    }
}

fn edit(message: EditorMessage) -> Message {
    Message::Editor(message)
}

fn create(agenda: &mut Agenda, kind: CardKind, title: &str, date: Option<&str>, time: &str) {
    send(
        agenda,
        [
            Message::OpenCreate(None),
            edit(EditorMessage::KindSelected(kind)),
            edit(EditorMessage::TitleChanged(title.into())),
            edit(EditorMessage::DatePicked(date.map(d))),
            edit(EditorMessage::TimeChanged(time.into())),
            Message::EditorSaved,
        ],
    );
}

fn titles(agenda: &Agenda, date: &str) -> Vec<String> {
    agenda
        .day(d(date))
        .map(|day| day.cards.iter().map(|c| c.title.clone()).collect())
        .unwrap_or_default()
}

fn first_card(agenda: &Agenda, date: &str) -> board::Card {
    agenda.day(d(date)).unwrap().cards[0].clone()
}

#[test]
fn board_spans_two_days_back_and_seven_ahead() {
    let (agenda, _) = setup();
    let days = agenda.days();
    assert_eq!(days.len(), 10);
    assert_eq!(days.first().unwrap().date, d("2026-10-19"));
    assert_eq!(days.last().unwrap().date, d("2026-10-28"));
}

#[test]
fn create_modal_adds_appointment_and_closes() {
    let (mut agenda, ctx) = setup();
    let mut events = ctx.events.subscribe();
    create(
        &mut agenda,
        CardKind::Appointment,
        "Reunião",
        Some("2026-10-22"),
        "10:00",
    );
    assert!(agenda.modal().is_none());
    let card = first_card(&agenda, "2026-10-22");
    assert_eq!(card.title, "Reunião");
    assert_eq!(card.kind, CardKind::Appointment);
    assert_eq!(card.time, NaiveTime::from_hms_opt(10, 0, 0));
    assert_eq!(events.try_recv().unwrap(), HubEvent::AgendaChanged);
}

#[test]
fn column_plus_opens_modal_on_that_day() {
    let (mut agenda, _) = setup();
    send(&mut agenda, [Message::OpenCreate(Some(d("2026-10-24")))]);
    assert_eq!(agenda.editor().unwrap().date, Some(d("2026-10-24")));
    send(
        &mut agenda,
        [
            edit(EditorMessage::TitleChanged("feira".into())),
            Message::EditorSaved,
        ],
    );
    assert_eq!(titles(&agenda, "2026-10-24"), ["feira"]);
}

#[test]
fn invalid_editor_keeps_modal_open_with_message() {
    let (mut agenda, _) = setup();
    create(
        &mut agenda,
        CardKind::Reminder,
        "água",
        Some("2026-10-21"),
        "",
    );
    let editor = agenda.editor().expect("modal continua aberto");
    assert!(editor.error.is_some());
    send(
        &mut agenda,
        [edit(EditorMessage::TimeChanged("18:00".into()))],
    );
    assert!(agenda.editor().unwrap().error.is_none());
    send(&mut agenda, [Message::EditorSaved]);
    assert!(agenda.modal().is_none());
    assert_eq!(first_card(&agenda, "2026-10-21").kind, CardKind::Reminder);
}

#[test]
fn reminder_notes_are_saved() {
    let (mut agenda, ctx) = setup();
    send(
        &mut agenda,
        [
            Message::OpenCreate(None),
            edit(EditorMessage::KindSelected(CardKind::Reminder)),
            edit(EditorMessage::TitleChanged("remédio".into())),
            edit(EditorMessage::TimeChanged("18:00".into())),
            edit(EditorMessage::NotesEdited(text_editor::Action::Edit(
                text_editor::Edit::Paste(Arc::new("tomar com água".into())),
            ))),
            Message::EditorSaved,
        ],
    );
    let CardId::Reminder(id) = first_card(&agenda, "2026-10-21").id else {
        panic!("esperava lembrete")
    };
    let reminder = agenda::get_reminder(ctx.db().conn(), id).unwrap();
    assert_eq!(reminder.notes, "tomar com água");
}

#[test]
fn opening_card_loads_details_and_saves_changes() {
    let (mut agenda, ctx) = setup();
    create(
        &mut agenda,
        CardKind::Appointment,
        "dentista",
        Some("2026-10-21"),
        "",
    );
    let card = first_card(&agenda, "2026-10-21");
    send(&mut agenda, [Message::OpenCard(card.id)]);
    let editor = agenda.editor().unwrap();
    assert!(!editor.is_new());
    assert_eq!(editor.title, "dentista");
    send(
        &mut agenda,
        [
            edit(EditorMessage::KindSelected(CardKind::Reminder)),
            edit(EditorMessage::TitleChanged("dentista às 15h".into())),
            edit(EditorMessage::DatePicked(Some(d("2026-10-23")))),
            edit(EditorMessage::PrioritySelected(Priority::High)),
            edit(EditorMessage::NotesEdited(text_editor::Action::Edit(
                text_editor::Edit::Paste(Arc::new("levar exames".into())),
            ))),
            Message::EditorSaved,
        ],
    );
    let CardId::Task(id) = card.id else {
        panic!("esperava tarefa")
    };
    let task = agenda::get_task(ctx.db().conn(), id).unwrap();
    assert_eq!(task.title, "dentista às 15h");
    assert_eq!(task.notes, "levar exames");
    assert_eq!(task.priority, Priority::High);
    assert_eq!(titles(&agenda, "2026-10-23"), ["dentista às 15h"]);
    assert_eq!(
        first_card(&agenda, "2026-10-23").kind,
        CardKind::Appointment
    );
}

#[test]
fn routine_is_created_from_modal() {
    let (mut agenda, _) = setup();
    send(
        &mut agenda,
        [
            Message::ScreenSelected(Screen::Routines),
            Message::OpenCreateRoutine,
            edit(EditorMessage::TitleChanged("remédio".into())),
            edit(EditorMessage::TimeChanged("20:00".into())),
            Message::EditorSaved,
        ],
    );
    assert!(agenda.modal().is_none());
    assert_eq!(agenda.routines()[0].recurrence, Recurrence::Daily);
    assert_eq!(first_card(&agenda, "2026-10-21").kind, CardKind::Routine);
}

#[test]
fn delete_asks_for_confirmation_first() {
    let (mut agenda, _) = setup();
    create(
        &mut agenda,
        CardKind::Appointment,
        "x",
        Some("2026-10-21"),
        "",
    );
    let card = first_card(&agenda, "2026-10-21");
    send(
        &mut agenda,
        [Message::AskDelete(
            DeleteTarget::Card(card.id),
            card.title.clone(),
        )],
    );
    assert!(matches!(agenda.modal(), Some(Modal::ConfirmDelete { .. })));
    assert_eq!(titles(&agenda, "2026-10-21"), ["x"]);
    send(&mut agenda, [Message::ModalClosed]);
    assert_eq!(titles(&agenda, "2026-10-21"), ["x"]);
    send(
        &mut agenda,
        [
            Message::AskDelete(DeleteTarget::Card(card.id), card.title),
            Message::DeleteConfirmed,
        ],
    );
    assert!(titles(&agenda, "2026-10-21").is_empty());
    assert!(agenda.modal().is_none());
}

#[test]
fn routine_delete_also_confirms() {
    let (mut agenda, ctx) = setup();
    let routine = agenda::create_routine(
        ctx.db().conn(),
        agenda::NewRoutine {
            title: "ler".into(),
            notes: String::new(),
            recurrence: Recurrence::Daily,
            time: None,
        },
    )
    .unwrap();
    send(&mut agenda, [Message::Refresh]);
    send(
        &mut agenda,
        [Message::AskDelete(
            DeleteTarget::Routine(routine.id),
            "ler".into(),
        )],
    );
    assert_eq!(agenda.routines().len(), 1);
    send(&mut agenda, [Message::DeleteConfirmed]);
    assert!(agenda.routines().is_empty());
}

#[test]
fn filter_selects_kinds_and_shift_click_resets() {
    let (mut agenda, _) = setup();
    assert!(!agenda.has_filter());
    assert!(agenda.shows(CardKind::Reminder));
    send(&mut agenda, [Message::FilterToggled(CardKind::Routine)]);
    assert!(agenda.shows(CardKind::Routine));
    assert!(!agenda.shows(CardKind::Appointment));
    send(&mut agenda, [Message::FilterToggled(CardKind::Reminder)]);
    assert!(agenda.shows(CardKind::Reminder));
    send(&mut agenda, [Message::FilterToggled(CardKind::Routine)]);
    assert!(!agenda.shows(CardKind::Routine));
    send(
        &mut agenda,
        [
            Message::ModifiersChanged(Modifiers::SHIFT),
            Message::FilterToggled(CardKind::Appointment),
        ],
    );
    assert!(!agenda.has_filter());
    send(
        &mut agenda,
        [
            Message::ModifiersChanged(Modifiers::default()),
            Message::FilterToggled(CardKind::Appointment),
        ],
    );
    assert!(agenda.is_filtered(CardKind::Appointment));
}

#[test]
fn completing_keeps_card_on_board_as_done() {
    let (mut agenda, _) = setup();
    create(
        &mut agenda,
        CardKind::Appointment,
        "regar",
        Some("2026-10-21"),
        "",
    );
    let CardId::Task(id) = first_card(&agenda, "2026-10-21").id else {
        panic!("esperava tarefa")
    };
    send(&mut agenda, [Message::TaskToggled(id, true)]);
    assert!(first_card(&agenda, "2026-10-21").done);
    assert_eq!(agenda.completed().len(), 1);
    send(&mut agenda, [Message::TaskToggled(id, false)]);
    assert!(!first_card(&agenda, "2026-10-21").done);
}

#[test]
fn old_and_undated_tasks_go_to_pending() {
    let (mut agenda, ctx) = setup();
    agenda::create_task(
        ctx.db().conn(),
        agenda::NewTask {
            title: "antiga".into(),
            date: Some(d("2026-10-10")),
            ..agenda::NewTask::default()
        },
        ctx.clock.now(),
    )
    .unwrap();
    create(&mut agenda, CardKind::Appointment, "comprar pão", None, "");
    let pending: Vec<&str> = agenda.pending().iter().map(|c| c.title.as_str()).collect();
    assert_eq!(pending, ["antiga", "comprar pão"]);
}

#[test]
fn navigation_moves_focus_and_returns_to_today() {
    let (mut agenda, _) = setup();
    send(&mut agenda, [Message::ShiftDays(7)]);
    assert_eq!(agenda.focus(), d("2026-10-28"));
    assert_eq!(agenda.days()[0].date, d("2026-10-26"));
    send(&mut agenda, [Message::GoToday]);
    assert_eq!(agenda.focus(), d("2026-10-21"));
}

#[test]
fn calendar_covers_month_and_opens_day() {
    let (mut agenda, _) = setup();
    create(
        &mut agenda,
        CardKind::Appointment,
        "aniversário",
        Some("2026-10-30"),
        "",
    );
    send(&mut agenda, [Message::ScreenSelected(Screen::Calendar)]);
    let grid = agenda.calendar();
    assert_eq!(grid.first().unwrap().date, d("2026-09-28"));
    let day = grid.iter().find(|day| day.date == d("2026-10-30")).unwrap();
    assert_eq!(day.cards[0].title, "aniversário");
    send(&mut agenda, [Message::MonthShifted(1)]);
    assert_eq!(agenda.month().month, 11);
    send(&mut agenda, [Message::DayOpened(d("2026-11-12"))]);
    assert_eq!(agenda.screen(), Screen::Week);
    assert_eq!(agenda.focus(), d("2026-11-12"));
}

#[test]
fn board_follows_the_clock_when_day_changes() {
    let clock = Arc::new(FakeClock::at("2026-10-21 23:59"));
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        clock.clone(),
    );
    let mut agenda = Agenda::new(ctx, Vec::new());
    clock.advance(TimeDelta::minutes(2));
    send(&mut agenda, [Message::Refresh]);
    assert_eq!(agenda.today(), d("2026-10-22"));
    assert_eq!(agenda.focus(), d("2026-10-22"));
}

#[test]
fn missed_reminders_can_be_dismissed() {
    let (_, ctx) = setup();
    let reminder = agenda::create_reminder(
        ctx.db().conn(),
        None,
        Some("velho"),
        NaiveDateTime::parse_from_str("2026-10-20 10:00", "%Y-%m-%d %H:%M").unwrap(),
    )
    .unwrap();
    let mut agenda = Agenda::new(ctx, vec![reminder]);
    assert_eq!(agenda.missed().len(), 1);
    send(&mut agenda, [Message::MissedDismissed]);
    assert!(agenda.missed().is_empty());
}

#[test]
fn view_builds_for_every_screen_and_modal() {
    let (mut agenda, _) = setup();
    create(
        &mut agenda,
        CardKind::Appointment,
        "a",
        Some("2026-10-21"),
        "18:00",
    );
    create(&mut agenda, CardKind::Appointment, "b", None, "");
    create(
        &mut agenda,
        CardKind::Reminder,
        "c",
        Some("2026-10-22"),
        "09:00",
    );
    send(&mut agenda, [Message::FilterToggled(CardKind::Reminder)]);
    for screen in [
        Screen::Week,
        Screen::Calendar,
        Screen::Routines,
        Screen::Completed,
    ] {
        send(&mut agenda, [Message::ScreenSelected(screen)]);
        let _ = agenda.view();
    }
    send(
        &mut agenda,
        [
            Message::OpenCreate(None),
            edit(EditorMessage::PickerToggled),
        ],
    );
    let _ = agenda.view();
    send(
        &mut agenda,
        [Message::ModalClosed, Message::OpenCreateRoutine],
    );
    let _ = agenda.view();
    let card = first_card(&agenda, "2026-10-21");
    send(
        &mut agenda,
        [Message::AskDelete(DeleteTarget::Card(card.id), card.title)],
    );
    let _ = agenda.view();
}
