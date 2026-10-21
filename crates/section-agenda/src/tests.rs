use std::sync::Arc;

use hub_core::agenda::{self, Recurrence};
use hub_core::chrono::{NaiveDateTime, NaiveTime};
use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

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

fn send(agenda: &mut Agenda, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = agenda.update(message);
    }
}

fn quick_add(agenda: &mut Agenda, text: &str) {
    send(
        agenda,
        [
            Message::QuickAddChanged(text.to_string()),
            Message::QuickAddSubmitted,
        ],
    );
}

#[test]
fn quick_add_creates_parsed_task_and_clears_field() {
    let (mut agenda, ctx) = setup();
    let mut events = ctx.events.subscribe();
    quick_add(&mut agenda, "pagar conta amanhã 10h");
    assert_eq!(agenda.quick_add_text(), "");
    assert!(agenda.today().today.is_empty());
    let (date, tasks) = &agenda.upcoming()[0];
    assert_eq!(date.to_string(), "2026-10-22");
    assert_eq!(tasks[0].title, "pagar conta");
    assert_eq!(tasks[0].time, NaiveTime::from_hms_opt(10, 0, 0));
    assert_eq!(events.try_recv().unwrap(), HubEvent::AgendaChanged);
}

#[test]
fn blank_quick_add_does_nothing() {
    let (mut agenda, _) = setup();
    quick_add(&mut agenda, "   ");
    assert!(agenda.today().undated.is_empty());
    assert!(agenda.error().is_none());
}

#[test]
fn unrecognized_text_goes_to_undated() {
    let (mut agenda, _) = setup();
    quick_add(&mut agenda, "comprar pão");
    assert_eq!(agenda.today().undated[0].title, "comprar pão");
}

#[test]
fn toggling_moves_task_between_today_and_completed() {
    let (mut agenda, _) = setup();
    quick_add(&mut agenda, "regar plantas hoje");
    let id = agenda.today().today[0].id;
    send(&mut agenda, [Message::TaskToggled(id, true)]);
    assert!(agenda.today().today.is_empty());
    assert_eq!(agenda.completed()[0].id, id);
    send(&mut agenda, [Message::TaskToggled(id, false)]);
    assert_eq!(agenda.today().today[0].id, id);
    assert!(agenda.completed().is_empty());
}

#[test]
fn deleting_task_removes_it() {
    let (mut agenda, _) = setup();
    quick_add(&mut agenda, "x");
    let id = agenda.today().undated[0].id;
    send(&mut agenda, [Message::TaskDeleted(id)]);
    assert!(agenda.today().undated.is_empty());
}

#[test]
fn deleting_missing_task_shows_error() {
    let (mut agenda, _) = setup();
    send(&mut agenda, [Message::TaskDeleted(42)]);
    assert!(agenda.error().is_some());
    send(&mut agenda, [Message::ErrorDismissed]);
    assert!(agenda.error().is_none());
}

#[test]
fn routine_form_creates_routine_and_todays_task() {
    let (mut agenda, _) = setup();
    send(
        &mut agenda,
        [
            Message::ScreenSelected(Screen::Routines),
            Message::RoutineFormOpened,
            Message::RoutineForm(RoutineFormMessage::TitleChanged("remédio".into())),
            Message::RoutineForm(RoutineFormMessage::TimeChanged("20h".into())),
            Message::RoutineSubmitted,
        ],
    );
    assert!(agenda.routine_form().is_none());
    assert_eq!(agenda.routines().len(), 1);
    assert_eq!(agenda.routines()[0].recurrence, Recurrence::Daily);
    assert_eq!(agenda.today().today[0].title, "remédio");
}

#[test]
fn invalid_routine_form_keeps_form_open_with_error() {
    let (mut agenda, _) = setup();
    send(
        &mut agenda,
        [
            Message::RoutineFormOpened,
            Message::RoutineForm(RoutineFormMessage::TitleChanged("aluguel".into())),
            Message::RoutineForm(RoutineFormMessage::KindSelected(RecurrenceKind::MonthDay)),
            Message::RoutineForm(RoutineFormMessage::NumberChanged("40".into())),
            Message::RoutineSubmitted,
        ],
    );
    assert!(agenda.routine_form().is_some());
    assert!(agenda.error().is_some());
    assert!(agenda.routines().is_empty());
}

#[test]
fn weekday_routine_uses_selected_days() {
    let form = {
        let mut form = RoutineForm::new(NaiveDateTime::default().date());
        form.update(RoutineFormMessage::TitleChanged("academia".into()));
        form.update(RoutineFormMessage::KindSelected(RecurrenceKind::Weekdays));
        for i in [1, 3, 4] {
            form.update(RoutineFormMessage::WeekdayToggled(i));
        }
        form
    };
    assert_eq!(form.build().unwrap().recurrence.to_string(), "semana:1,3");
}

#[test]
fn disabling_routine_keeps_it_listed() {
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
    send(&mut agenda, [Message::RoutineToggled(routine.id, false)]);
    assert!(!agenda.routines()[0].active);
    send(&mut agenda, [Message::RoutineToggled(routine.id, true)]);
    assert!(agenda.routines()[0].active);
    assert_eq!(agenda.today().today.len(), 1);
}

#[test]
fn refresh_picks_up_external_changes() {
    let (mut agenda, ctx) = setup();
    agenda::create_task(
        ctx.db().conn(),
        agenda::NewTask {
            title: "criada pela IA".into(),
            ..agenda::NewTask::default()
        },
        ctx.clock.now(),
    )
    .unwrap();
    assert!(agenda.today().undated.is_empty());
    send(&mut agenda, [Message::Refresh]);
    assert_eq!(agenda.today().undated.len(), 1);
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
fn view_builds_for_every_screen() {
    let (mut agenda, _) = setup();
    quick_add(&mut agenda, "a hoje 18h");
    quick_add(&mut agenda, "b amanhã");
    for screen in [
        Screen::Today,
        Screen::Upcoming,
        Screen::Routines,
        Screen::Completed,
    ] {
        send(&mut agenda, [Message::ScreenSelected(screen)]);
        let _ = agenda.view();
    }
    send(&mut agenda, [Message::RoutineFormOpened]);
    let _ = agenda.view();
}
