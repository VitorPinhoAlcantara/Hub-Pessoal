use std::sync::Arc;

use hub_core::agenda;
use hub_core::clock::{Clock, FakeClock};
use hub_core::platform::{FakePlatform, PlatformCall};
use hub_core::{Config, Database, HubPaths};

use super::*;

fn setup() -> (App, Arc<FakePlatform>, Arc<FakeClock>) {
    let platform = Arc::new(FakePlatform::default());
    let clock = Arc::new(FakeClock::at("2026-10-21 09:00"));
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform.clone(),
        clock.clone(),
    );
    let (app, _) = App::boot(ctx, Options::default());
    (app, platform, clock)
}

fn open(app: &mut App) -> window::Id {
    let id = app.window.expect("janela pedida no boot");
    let _ = app.update(Message::WindowOpened(id));
    id
}

#[test]
fn boot_spawns_tray_and_requests_window() {
    let (app, platform, _) = setup();
    assert!(platform.calls().contains(&PlatformCall::SpawnTray));
    assert!(app.tray_available);
    assert!(app.window.is_some());
    assert!(app.agenda.is_none());
}

#[test]
fn agenda_is_created_lazily_when_window_opens() {
    let (mut app, _, _) = setup();
    open(&mut app);
    assert!(app.agenda.is_some());
}

#[test]
fn other_tabs_do_not_create_agenda() {
    let (mut app, _, _) = setup();
    let _ = app.update(Message::TabSelected(Tab::Scripts));
    assert!(app.agenda.is_none());
    let _ = app.update(Message::TabSelected(Tab::Agenda));
    assert!(app.agenda.is_some());
}

#[test]
fn closing_window_releases_state_but_keeps_running() {
    let (mut app, _, _) = setup();
    let id = open(&mut app);
    let _ = app.update(Message::WindowClosed(id));
    assert!(app.window.is_none());
    assert!(app.agenda.is_none());
}

#[test]
fn tray_open_reopens_window() {
    let (mut app, platform, _) = setup();
    let id = open(&mut app);
    let _ = app.update(Message::WindowClosed(id));
    assert!(platform.click_tray(TrayEvent::Open));
    let _ = app.update(Message::Bus(HubEvent::ShowWindow));
    assert!(app.window.is_some());
    assert_ne!(app.window, Some(id));
}

#[test]
fn scheduler_wake_fires_reminder_and_moves_next_wake() {
    let (mut app, platform, clock) = setup();
    agenda::create_reminder(
        app.ctx.db().conn(),
        None,
        Some("beber água"),
        clock.now() + hub_core::chrono::TimeDelta::minutes(1),
    )
    .unwrap();
    let _ = app.update(Message::Bus(HubEvent::AgendaChanged));
    assert_eq!(
        app.next_wake,
        clock.now() + hub_core::chrono::TimeDelta::minutes(1)
    );
    clock.advance(hub_core::chrono::TimeDelta::minutes(1));
    let _ = app.update(Message::SchedulerWake);
    assert_eq!(platform.notifications().len(), 1);
    assert_eq!(platform.notifications()[0].title, "beber água");
    assert!(app.next_wake > clock.now());
}

#[test]
fn missed_reminders_reach_agenda() {
    let platform = Arc::new(FakePlatform::default());
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform,
        Arc::new(FakeClock::at("2026-10-21 09:00")),
    );
    agenda::create_reminder(
        ctx.db().conn(),
        None,
        Some("velho"),
        hub_core::chrono::NaiveDateTime::parse_from_str("2026-10-20 10:00", "%Y-%m-%d %H:%M")
            .unwrap(),
    )
    .unwrap();
    let (mut app, _) = App::boot(ctx, Options::default());
    open(&mut app);
    assert_eq!(app.agenda.as_ref().unwrap().missed().len(), 1);
    assert!(app.missed.is_empty());
}

#[test]
fn view_builds_for_every_tab_and_panel() {
    let (mut app, _, _) = setup();
    let id = open(&mut app);
    for tab in [
        Tab::Agenda,
        Tab::Scripts,
        Tab::Shortcuts,
        Tab::WhatsApp,
        Tab::Settings,
    ] {
        let _ = app.update(Message::TabSelected(tab));
        let _ = app.update(Message::ToggleAi);
        let _ = app.update(Message::TogglePlayer);
        let _ = app.view(id);
    }
    let _ = app.update(Message::MaximizedChanged(true));
    let _ = app.view(id);
}
