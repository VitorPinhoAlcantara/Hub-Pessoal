use std::sync::Arc;

use hub_core::agenda::{self, NewTask};
use hub_core::chrono::{NaiveDate, NaiveTime};
use hub_core::clock::FakeClock;
use hub_core::notes;
use hub_core::per_os::PerOs;
use hub_core::platform::{FakePlatform, Os, PlatformCall};
use hub_core::shortcuts::{self, Shortcut, ShortcutKind};
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

fn setup() -> (Home, HubContext, Arc<FakePlatform>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    let platform = Arc::new(FakePlatform::default());
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform.clone(),
        Arc::new(FakeClock::at("2026-10-22 10:00")),
    );
    (Home::new(ctx.clone()), ctx, platform, temp)
}

fn add_shortcut(ctx: &HubContext, name: &str, url: &str) {
    let shortcut = Shortcut {
        name: name.into(),
        kind: ShortcutKind::Url,
        target: PerOs::Same(url.into()),
        group: "Geral".into(),
    };
    ctx.shortcut_store()
        .add(&ctx.paths.shortcuts_file(), shortcut)
        .unwrap();
}

#[test]
fn empty_home_has_no_content() {
    let (home, _, _, _temp) = setup();
    assert_eq!(home.title(), "Início");
    assert!(home.items().is_empty());
    assert_eq!(home.overdue(), 0);
    assert!(home.most_used().is_empty());
    assert!(home.favorite().is_none());
    let _ = home.view();
}

#[test]
fn shows_today_tasks_and_counts_overdue() {
    let (mut home, ctx, _, _temp) = setup();
    let now = ctx.clock.now();
    let today = now.date();
    {
        let db = ctx.db();
        for (title, date, time) in [
            ("Dentista", today, NaiveTime::from_hms_opt(15, 0, 0)),
            ("Sem horário", today, None),
            ("Reunião", today, NaiveTime::from_hms_opt(9, 30, 0)),
            ("Atrasada", today.pred_opt().unwrap(), None),
        ] {
            agenda::create_task(
                db.conn(),
                NewTask {
                    title: title.into(),
                    date: Some(date),
                    time,
                    ..NewTask::default()
                },
                now,
            )
            .unwrap();
        }
        agenda::create_reminder(
            db.conn(),
            None,
            Some("Beber água"),
            today.and_hms_opt(11, 0, 0).unwrap(),
        )
        .unwrap();
    }
    home.refresh();
    let titles: Vec<&str> = home.items().iter().map(|i| i.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Reunião",
            "Beber água",
            "Dentista",
            "Sem horário",
            "Atrasada"
        ]
    );
    assert_eq!(home.items()[0].label, "09:30");
    assert_eq!(home.items()[1].kind, ItemKind::Reminder);
    assert_eq!(home.items()[3].label, "Sem hora");
    assert!(home.items()[4].overdue);
    assert_eq!(home.overdue(), 1);
    let _ = home.view();
}

#[test]
fn agenda_is_limited_to_six_items() {
    let (mut home, ctx, _, _temp) = setup();
    let now = ctx.clock.now();
    for index in 0..9 {
        agenda::create_task(
            ctx.db().conn(),
            NewTask {
                title: format!("Tarefa {index}"),
                date: Some(now.date()),
                ..NewTask::default()
            },
            now,
        )
        .unwrap();
    }
    home.refresh();
    assert_eq!(home.items().len(), AGENDA_LIMIT);
}

#[test]
fn shows_favorite_note() {
    let (mut home, ctx, _, _temp) = setup();
    let note = {
        let db = ctx.db();
        let note = notes::create(db.conn(), "Lista", "um\ndois", ctx.clock.now()).unwrap();
        notes::set_favorite(db.conn(), note.id, true).unwrap();
        note
    };
    home.refresh();
    let favorite = home.favorite().unwrap();
    assert_eq!(favorite.id, note.id);
    assert_eq!(favorite.display_title(), "Lista");
    let _ = home.view();
}

#[test]
fn clicking_shortcut_calls_platform_and_records_use() {
    let (mut home, ctx, platform, _temp) = setup();
    add_shortcut(&ctx, "Site", "exemplo.com");
    add_shortcut(&ctx, "Outro", "outro.com");
    home.refresh();
    assert_eq!(home.most_used().len(), 2);
    let _ = home.update(Message::Open(1));
    assert_eq!(
        platform.calls(),
        vec![PlatformCall::OpenUrl("https://outro.com".into())]
    );
    let usage = shortcuts::usage(ctx.db().conn()).unwrap();
    assert_eq!(usage["url|outro.com"].0, 1);
    assert_eq!(home.most_used()[0].name, "Outro");
    assert!(home.error().is_none());
}

#[test]
fn failed_open_shows_dismissible_error() {
    let (mut home, ctx, platform, _temp) = setup();
    let target = match ctx.platform.os() {
        Os::Windows => PerOs::split(None, Some("code".into())),
        Os::Linux => PerOs::split(Some("code".into()), None),
    };
    let shortcut = Shortcut {
        name: "Outro sistema".into(),
        kind: ShortcutKind::Program,
        target,
        group: "Geral".into(),
    };
    ctx.shortcut_store()
        .add(&ctx.paths.shortcuts_file(), shortcut)
        .unwrap();
    home.refresh();
    let _ = home.update(Message::Open(0));
    assert!(home.error().unwrap().contains("Outro sistema"));
    assert!(platform.calls().is_empty());
    assert!(shortcuts::usage(ctx.db().conn()).unwrap().is_empty());
    let _ = home.update(Message::ErrorDismissed);
    assert!(home.error().is_none());
}

#[test]
fn formats_dates_in_portuguese() {
    let date = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    assert_eq!(
        format_date(date(2026, 10, 22)),
        "quinta feira, 22 de outubro de 2026"
    );
    assert_eq!(format_date(date(2026, 3, 1)), "domingo, 1 de março de 2026");
    assert_eq!(
        format_date(date(2026, 12, 25)),
        "sexta feira, 25 de dezembro de 2026"
    );
    assert_eq!(
        format_date(date(2026, 2, 7)),
        "sábado, 7 de fevereiro de 2026"
    );
}

#[test]
fn navigate_does_not_change_state() {
    let (mut home, ctx, _, _temp) = setup();
    add_shortcut(&ctx, "Site", "exemplo.com");
    home.refresh();
    let before = (home.items().to_vec(), home.most_used().to_vec(), home.now());
    for destination in [
        Destination::Agenda,
        Destination::Shortcuts,
        Destination::Notes(Some(3)),
        Destination::Notes(None),
    ] {
        let _ = home.update(Message::Navigate(destination));
    }
    let after = (home.items().to_vec(), home.most_used().to_vec(), home.now());
    assert_eq!(before, after);
    assert!(home.error().is_none());
}

#[test]
fn tick_advances_clock() {
    let (mut home, _, _, _temp) = setup();
    assert_eq!(home.now(), home.ctx.clock.now());
    let _ = home.update(Message::Tick);
    assert_eq!(home.now(), home.ctx.clock.now());
}
