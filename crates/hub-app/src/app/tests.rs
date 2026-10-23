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
fn home_is_created_lazily_when_window_opens() {
    let (mut app, _, _) = setup();
    assert!(app.home.is_none());
    open(&mut app);
    assert!(app.home.is_some());
    assert!(app.agenda.is_none());
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

#[test]
fn confirmation_request_reopens_window_and_answer_reaches_tool() {
    let (mut app, _, _) = setup();
    let id = open(&mut app);
    let _ = app.update(Message::WindowClosed(id));
    let (request_id, mut answer) =
        app.ctx
            .confirmations
            .ask(&app.ctx.events, "Executar backup?", "pedido da IA");
    let request = app.ctx.confirmations.pending()[0].clone();
    let _ = app.update(Message::Bus(HubEvent::ConfirmationRequested(request)));
    assert!(app.window.is_some());
    assert_eq!(app.confirmations.len(), 1);
    let window = app.window.unwrap();
    let _ = app.view(window);
    let _ = app.update(Message::ConfirmAnswered(request_id, true));
    assert!(app.confirmations.is_empty());
    assert_eq!(answer.try_recv(), Ok(true));
}

#[test]
fn closed_confirmation_leaves_queue() {
    let (mut app, _, _) = setup();
    let (request_id, _answer) = app.ctx.confirmations.ask(&app.ctx.events, "x", "y");
    let request = app.ctx.confirmations.pending()[0].clone();
    let _ = app.update(Message::Bus(HubEvent::ConfirmationRequested(request)));
    let _ = app.update(Message::Bus(HubEvent::ConfirmationClosed(request_id)));
    assert!(app.confirmations.is_empty());
}

#[test]
fn ai_panel_is_created_on_demand_without_mcp_in_tests() {
    let (mut app, _, _) = setup();
    assert!(app.chat.is_none());
    assert!(app.mcp.is_err());
    let id = open(&mut app);
    let _ = app.update(Message::ToggleAi);
    assert!(app.chat.is_some());
    let _ = app.view(id);
    let _ = app.update(Message::WindowClosed(id));
    assert!(app.chat.is_none());
}

#[test]
fn play_request_opens_player_and_keeps_it_after_window_closes() {
    let (mut app, _, _) = setup();
    let id = open(&mut app);
    assert!(app.player.is_none());
    let _ = app.update(Message::Bus(HubEvent::PlayRequested(
        "https://www.youtube.com/playlist?list=abc".into(),
    )));
    assert!(app.player_open);
    assert!(app.player.is_some());
    let _ = app.view(id);
    let _ = app.update(Message::WindowClosed(id));
    assert!(app.player.is_some());
    let _ = app.update(Message::TogglePlayer);
    assert!(!app.player_open);
}

#[test]
fn player_control_from_bus_reaches_player() {
    let (mut app, _, _) = setup();
    let _ = app.update(Message::Bus(HubEvent::PlayerControl(
        hub_core::player::PlayerCommand::SetVolume(40),
    )));
    let player = app.player.as_ref().unwrap();
    assert_eq!(player.status().volume, 40.0);
    let _ = app.update(Message::Bus(HubEvent::PlayerControl(
        hub_core::player::PlayerCommand::Pause,
    )));
    let _ = app.update(Message::Bus(HubEvent::PlayerControl(
        hub_core::player::PlayerCommand::FavoritesChanged,
    )));
    assert!(!app.player_open);
}

#[test]
fn whatsapp_tab_is_heavy_and_released_on_hide() {
    let (mut app, _, _) = setup();
    let id = open(&mut app);
    let _ = app.update(Message::TabSelected(Tab::WhatsApp));
    assert!(app.whatsapp.is_some());
    let _ = app.view(id);
    let _ = app.update(Message::TabSelected(Tab::Agenda));
    assert!(app.whatsapp.is_none());
    assert!(app.ctx.whatsapp.get().is_none());
}
