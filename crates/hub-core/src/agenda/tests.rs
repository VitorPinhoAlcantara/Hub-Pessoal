use super::*;
use crate::Database;

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn new_task(title: &str, date: Option<&str>, time: Option<&str>) -> NewTask {
    NewTask {
        title: title.to_string(),
        date: date.map(day),
        time: time.map(|t| NaiveTime::parse_from_str(t, "%H:%M").unwrap()),
        ..NewTask::default()
    }
}

fn db() -> Database {
    Database::open_in_memory().unwrap()
}

#[test]
fn create_and_read_task() {
    let db = db();
    let now = at("2026-10-21 09:00");
    let task = create_task(
        db.conn(),
        new_task("  pagar conta ", Some("2026-10-22"), None),
        now,
    )
    .unwrap();
    assert_eq!(task.title, "pagar conta");
    assert_eq!(task.created_at, now);
    assert_eq!(get_task(db.conn(), task.id).unwrap(), task);
}

#[test]
fn empty_title_is_rejected() {
    let db = db();
    let result = create_task(
        db.conn(),
        new_task("   ", None, None),
        at("2026-10-21 09:00"),
    );
    assert!(matches!(result, Err(AgendaError::EmptyTitle)));
}

#[test]
fn task_with_date_and_time_gets_reminder() {
    let db = db();
    let now = at("2026-10-21 09:00");
    create_task(
        db.conn(),
        new_task("a", Some("2026-10-21"), Some("10:00")),
        now,
    )
    .unwrap();
    create_task(db.conn(), new_task("b", Some("2026-10-21"), None), now).unwrap();
    assert_eq!(
        next_reminder_time(db.conn()).unwrap(),
        Some(at("2026-10-21 10:00"))
    );
    assert!(due_reminders(db.conn(), now).unwrap().is_empty());
    let due = due_reminders(db.conn(), at("2026-10-21 10:00")).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].title, "a");
}

#[test]
fn today_view_groups_overdue_today_and_undated() {
    let db = db();
    let now = at("2026-10-21 09:00");
    let c = db.conn();
    create_task(c, new_task("ontem", Some("2026-10-20"), None), now).unwrap();
    create_task(
        c,
        new_task("hoje tarde", Some("2026-10-21"), Some("15:00")),
        now,
    )
    .unwrap();
    create_task(
        c,
        new_task("hoje cedo", Some("2026-10-21"), Some("08:00")),
        now,
    )
    .unwrap();
    create_task(c, new_task("hoje sem hora", Some("2026-10-21"), None), now).unwrap();
    create_task(c, new_task("sem data", None, None), now).unwrap();
    create_task(c, new_task("amanha", Some("2026-10-22"), None), now).unwrap();
    let done = create_task(c, new_task("feita", Some("2026-10-21"), None), now).unwrap();
    complete_task(c, done.id, now).unwrap();

    let view = today_view(c, day("2026-10-21")).unwrap();
    let titles = |tasks: &[Task]| tasks.iter().map(|t| t.title.clone()).collect::<Vec<_>>();
    assert_eq!(titles(&view.overdue), ["ontem"]);
    assert_eq!(
        titles(&view.today),
        ["hoje cedo", "hoje tarde", "hoje sem hora"]
    );
    assert_eq!(titles(&view.undated), ["sem data"]);
}

#[test]
fn upcoming_groups_by_day_within_range() {
    let db = db();
    let now = at("2026-10-21 09:00");
    let c = db.conn();
    for (title, date) in [
        ("hoje", "2026-10-21"),
        ("a", "2026-10-22"),
        ("b", "2026-10-22"),
        ("c", "2026-11-04"),
        ("longe", "2026-11-05"),
    ] {
        create_task(c, new_task(title, Some(date), None), now).unwrap();
    }
    let groups = upcoming(c, day("2026-10-21"), 14).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, day("2026-10-22"));
    assert_eq!(groups[0].1.len(), 2);
    assert_eq!(groups[1].0, day("2026-11-04"));
}

#[test]
fn complete_and_undo() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    let task = create_task(c, new_task("x", Some("2026-10-21"), Some("10:00")), now).unwrap();
    complete_task(c, task.id, at("2026-10-21 09:30")).unwrap();
    assert!(get_task(c, task.id).unwrap().is_completed());
    assert!(due_reminders(c, at("2026-10-21 11:00")).unwrap().is_empty());
    assert_eq!(completed(c, 10).unwrap().len(), 1);
    uncomplete_task(c, task.id).unwrap();
    assert!(!get_task(c, task.id).unwrap().is_completed());
    assert!(completed(c, 10).unwrap().is_empty());
}

#[test]
fn missing_ids_return_not_found() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    assert!(matches!(
        complete_task(c, 99, now),
        Err(AgendaError::NotFound(99))
    ));
    assert!(matches!(delete_task(c, 99), Err(AgendaError::NotFound(99))));
    assert!(matches!(
        set_routine_active(c, 99, false),
        Err(AgendaError::NotFound(99))
    ));
}

#[test]
fn delete_task_removes_its_reminders() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    let task = create_task(c, new_task("x", Some("2026-10-21"), Some("10:00")), now).unwrap();
    delete_task(c, task.id).unwrap();
    assert_eq!(next_reminder_time(c).unwrap(), None);
}

#[test]
fn update_task_moves_reminder() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    let task = create_task(c, new_task("x", Some("2026-10-21"), Some("10:00")), now).unwrap();
    update_task(c, task.id, new_task("y", Some("2026-10-22"), Some("11:00"))).unwrap();
    assert_eq!(next_reminder_time(c).unwrap(), Some(at("2026-10-22 11:00")));
    update_task(c, task.id, new_task("y", None, None)).unwrap();
    assert_eq!(next_reminder_time(c).unwrap(), None);
}

#[test]
fn routines_generate_one_task_per_day() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 06:00");
    let routine = create_routine(
        c,
        NewRoutine {
            title: "remédio".into(),
            notes: String::new(),
            recurrence: Recurrence::Daily,
            time: NaiveTime::from_hms_opt(8, 0, 0),
        },
    )
    .unwrap();
    create_routine(
        c,
        NewRoutine {
            title: "academia".into(),
            notes: String::new(),
            recurrence: "semana:1".parse().unwrap(),
            time: None,
        },
    )
    .unwrap();
    assert_eq!(
        generate_routine_tasks(c, day("2026-10-21"), now).unwrap(),
        1
    );
    assert_eq!(
        generate_routine_tasks(c, day("2026-10-21"), now).unwrap(),
        0
    );
    let view = today_view(c, day("2026-10-21")).unwrap();
    assert_eq!(view.today.len(), 1);
    assert_eq!(view.today[0].routine_id, Some(routine.id));
    assert_eq!(next_reminder_time(c).unwrap(), Some(at("2026-10-21 08:00")));

    set_routine_active(c, routine.id, false).unwrap();
    assert_eq!(
        generate_routine_tasks(c, day("2026-10-22"), now).unwrap(),
        0
    );
    assert_eq!(
        generate_routine_tasks(c, day("2026-10-26"), now).unwrap(),
        1
    );
}

#[test]
fn loose_reminder_needs_title() {
    let db = db();
    let c = db.conn();
    assert!(matches!(
        create_reminder(c, None, Some(" "), at("2026-10-21 10:00")),
        Err(AgendaError::EmptyTitle)
    ));
    let reminder = create_reminder(c, None, Some("beber água"), at("2026-10-21 10:00")).unwrap();
    assert_eq!(reminder.title, "beber água");
    assert_eq!(reminder.task_id, None);
}

#[test]
fn snooze_and_fire() {
    let db = db();
    let c = db.conn();
    let r = create_reminder(c, None, Some("x"), at("2026-10-21 10:00")).unwrap();
    mark_reminder_fired(c, r.id, at("2026-10-21 10:00")).unwrap();
    assert_eq!(next_reminder_time(c).unwrap(), None);
    snooze_reminder(c, r.id, at("2026-10-21 10:10")).unwrap();
    assert_eq!(next_reminder_time(c).unwrap(), Some(at("2026-10-21 10:10")));
    assert!(due_reminders(c, at("2026-10-21 10:05")).unwrap().is_empty());
    assert_eq!(due_reminders(c, at("2026-10-21 10:10")).unwrap().len(), 1);
}

#[test]
fn missed_reminders_are_taken_once() {
    let db = db();
    let c = db.conn();
    create_reminder(c, None, Some("velho"), at("2026-10-20 10:00")).unwrap();
    create_reminder(c, None, Some("agora"), at("2026-10-21 09:59")).unwrap();
    create_reminder(c, None, Some("futuro"), at("2026-10-21 12:00")).unwrap();
    let now = at("2026-10-21 10:00");
    let missed = take_missed_reminders(c, now, TimeDelta::minutes(2)).unwrap();
    assert_eq!(missed.len(), 1);
    assert_eq!(missed[0].title, "velho");
    assert!(
        take_missed_reminders(c, now, TimeDelta::minutes(2))
            .unwrap()
            .is_empty()
    );
    assert_eq!(due_reminders(c, now).unwrap().len(), 1);
}

#[test]
fn pending_before_lists_old_open_and_undated_tasks() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    create_task(c, new_task("antiga", Some("2026-10-10"), None), now).unwrap();
    create_task(c, new_task("recente", Some("2026-10-19"), None), now).unwrap();
    create_task(c, new_task("sem data", None, None), now).unwrap();
    let done = create_task(c, new_task("feita", Some("2026-10-01"), None), now).unwrap();
    complete_task(c, done.id, now).unwrap();
    let titles: Vec<String> = pending_before(c, day("2026-10-18"))
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, ["antiga", "sem data"]);
}

#[test]
fn loose_reminders_between_ignores_task_reminders() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    create_task(
        c,
        new_task("tarefa", Some("2026-10-21"), Some("10:00")),
        now,
    )
    .unwrap();
    create_reminder(c, None, Some("dentro"), at("2026-10-22 23:59")).unwrap();
    create_reminder(c, None, Some("fora"), at("2026-10-23 00:00")).unwrap();
    let found = loose_reminders_between(c, day("2026-10-21"), day("2026-10-22")).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "dentro");
    delete_reminder(c, found[0].id).unwrap();
    assert!(
        loose_reminders_between(c, day("2026-10-21"), day("2026-10-22"))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        delete_reminder(c, 999),
        Err(AgendaError::NotFound(999))
    ));
}

#[test]
fn update_reminder_changes_text_and_rearms_when_moved_forward() {
    let db = db();
    let c = db.conn();
    let r = create_reminder(c, None, Some("x"), at("2026-10-21 10:00")).unwrap();
    mark_reminder_fired(c, r.id, at("2026-10-21 10:00")).unwrap();
    let now = at("2026-10-21 11:00");
    let same = update_reminder(c, r.id, "y", "detalhes", at("2026-10-21 10:00"), now).unwrap();
    assert_eq!(same.title, "y");
    assert_eq!(same.notes, "detalhes");
    assert!(same.fired_at.is_some());
    let moved = update_reminder(c, r.id, "y", "", at("2026-10-21 12:00"), now).unwrap();
    assert!(moved.fired_at.is_none());
    assert_eq!(next_reminder_time(c).unwrap(), Some(at("2026-10-21 12:00")));
    assert!(matches!(
        update_reminder(c, r.id, " ", "", at("2026-10-21 12:00"), now),
        Err(AgendaError::EmptyTitle)
    ));
}

#[test]
fn projections_show_future_routine_days_without_duplicates() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    create_routine(
        c,
        NewRoutine {
            title: "dia sim dia não".into(),
            notes: String::new(),
            recurrence: "cada:2:2026-10-21".parse().unwrap(),
            time: NaiveTime::from_hms_opt(7, 0, 0),
        },
    )
    .unwrap();
    let paused = create_routine(
        c,
        NewRoutine {
            title: "pausada".into(),
            notes: String::new(),
            recurrence: Recurrence::Daily,
            time: None,
        },
    )
    .unwrap();
    set_routine_active(c, paused.id, false).unwrap();
    generate_routine_tasks(c, day("2026-10-21"), now).unwrap();
    let list =
        routine_projections(c, day("2026-10-19"), day("2026-10-27"), day("2026-10-21")).unwrap();
    let dates: Vec<String> = list.iter().map(|p| p.date.to_string()).collect();
    assert_eq!(dates, ["2026-10-23", "2026-10-25", "2026-10-27"]);
    assert!(list.iter().all(|p| p.routine.title == "dia sim dia não"));
}

#[test]
fn moving_keeps_time_and_updates_reminders() {
    let db = db();
    let c = db.conn();
    let now = at("2026-10-21 09:00");
    let task = create_task(c, new_task("x", Some("2026-10-21"), Some("15:00")), now).unwrap();
    let moved = move_task(c, task.id, Some(day("2026-10-24"))).unwrap();
    assert_eq!(moved.time, task.time);
    assert_eq!(next_reminder_time(c).unwrap(), Some(at("2026-10-24 15:00")));
    let r = create_reminder(c, None, Some("r"), at("2026-10-22 08:30")).unwrap();
    let moved = move_reminder(c, r.id, day("2026-10-26"), now).unwrap();
    assert_eq!(moved.effective_time(), at("2026-10-26 08:30"));
    assert_eq!(pending_reminders(c).unwrap().len(), 2);
}

#[test]
fn routines_can_be_updated() {
    let db = db();
    let c = db.conn();
    let routine = create_routine(
        c,
        NewRoutine {
            title: "a".into(),
            notes: String::new(),
            recurrence: Recurrence::Daily,
            time: None,
        },
    )
    .unwrap();
    let updated = update_routine(
        c,
        routine.id,
        NewRoutine {
            title: "b".into(),
            notes: "n".into(),
            recurrence: "semana:2".parse().unwrap(),
            time: NaiveTime::from_hms_opt(6, 0, 0),
        },
    )
    .unwrap();
    assert_eq!(updated.title, "b");
    assert_eq!(updated.recurrence.to_string(), "semana:2");
    assert!(matches!(
        update_routine(
            c,
            99,
            NewRoutine {
                title: "x".into(),
                notes: String::new(),
                recurrence: Recurrence::Daily,
                time: None
            }
        ),
        Err(AgendaError::NotFound(99))
    ));
}
