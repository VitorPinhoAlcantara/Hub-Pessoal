use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::leds::{Device, DeviceKind, Mode, Snapshot};
use hub_core::platform::FakePlatform;
use hub_core::{Config, Database, HubContext, HubPaths};
use hub_ui_kit::Section;

use super::*;

fn setup() -> (Peripherals, HubContext, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    Config::default().save(&paths.config_file()).unwrap();
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    (Peripherals::new(ctx.clone()), ctx, temp)
}

fn mode(name: &str, flags: u32) -> Mode {
    Mode {
        name: name.to_string(),
        value: 0,
        flags,
        speed_min: 1,
        speed_max: 5,
        brightness_min: 0,
        brightness_max: 0,
        colors_min: 0,
        colors_max: 0,
        speed: 2,
        brightness: 0,
        direction: 0,
        color_mode: 0,
        colors: Vec::new(),
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        devices: vec![Device {
            index: 0,
            kind: DeviceKind::Keyboard,
            name: "Teclado".to_string(),
            vendor: String::new(),
            description: String::new(),
            serial: String::new(),
            location: String::new(),
            active_mode: 0,
            modes: vec![mode("Direct", 1 << 5), mode("Breathing", 1)],
            led_count: 4,
        }],
        profiles: vec!["Jogo".to_string(), "Normal".to_string()],
    }
}

#[test]
fn starts_loading_and_renders_every_state() {
    let (mut section, _ctx, _temp) = setup();
    assert_eq!(section.title(), "Periféricos");
    assert_eq!(*section.status(), Status::Loading);
    let _ = section.view();
    let _ = section.update(Message::Loaded(Err("recusado".to_string())));
    assert!(matches!(section.status(), Status::Failed(_)));
    let _ = section.view();
    let _ = section.update(Message::Loaded(Ok(snapshot())));
    assert!(matches!(section.status(), Status::Ready(_)));
    let _ = section.view();
    let _ = section.update(Message::Focus(Focus::Device(0)));
    assert_eq!(section.focus(), Focus::Device(0));
    let _ = section.update(Message::Mode(0, 1));
    let _ = section.view();
    match section.status() {
        Status::Ready(snapshot) => assert_eq!(snapshot.devices[0].active_mode, 1),
        other => panic!("estado inesperado: {other:?}"),
    }
}

#[test]
fn profile_choices_are_saved_to_config() {
    let (mut section, ctx, _temp) = setup();
    let _ = section.update(Message::GameProfile("Jogo".to_string()));
    let _ = section.update(Message::NormalProfile("Normal".to_string()));
    let leds = ctx.config().leds;
    assert_eq!(leds.game_profile, "Jogo");
    assert_eq!(leds.normal_profile, "Normal");
    let _ = section.update(Message::GameProfile(view::NONE_LABEL.to_string()));
    assert!(ctx.config().leds.game_profile.is_empty());
    let saved = Config::load(&ctx.paths.config_file()).unwrap();
    assert_eq!(saved.leds.normal_profile, "Normal");
}

#[test]
fn painting_while_busy_keeps_only_the_latest_color() {
    let (mut section, _ctx, _temp) = setup();
    let _ = section.update(Message::Swatch(Focus::All, color_picker::SWATCHES[1]));
    let _ = section.update(Message::Swatch(Focus::All, color_picker::SWATCHES[2]));
    let _ = section.update(Message::Swatch(Focus::All, color_picker::SWATCHES[3]));
    assert!(section.pending.is_some());
    assert_eq!(section.all_color(), color_picker::SWATCHES[3]);
}
