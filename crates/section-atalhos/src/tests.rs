use std::path::PathBuf;
use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::{FakePlatform, FileIcon, PlatformCall};
use hub_core::{Config, Database, HubPaths};

use super::*;

fn setup(apps: Vec<InstalledApp>) -> (Shortcuts, Arc<FakePlatform>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    let platform = Arc::new(FakePlatform::new(Os::Windows).with_apps(apps));
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform.clone(),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    (Shortcuts::new(ctx), platform, temp)
}

fn send(section: &mut Shortcuts, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = section.update(message);
    }
}

fn add(section: &mut Shortcuts, name: &str, kind: ShortcutKind, target: &str, group: &str) {
    send(
        section,
        [
            Message::NewShortcut,
            Message::Form(FormMessage::NameChanged(name.into())),
            Message::Form(FormMessage::KindSelected(kind)),
            Message::Form(FormMessage::WindowsChanged(target.into())),
            Message::Form(FormMessage::LinuxChanged(target.into())),
            Message::Form(FormMessage::GroupChanged(group.into())),
            Message::FormSaved,
        ],
    );
}

fn app(name: &str, target: &str) -> InstalledApp {
    InstalledApp {
        name: name.into(),
        target: target.into(),
    }
}

fn menu_labels(section: &Shortcuts) -> Vec<String> {
    section
        .context_menu()
        .map(|menu| menu.items.into_iter().map(|item| item.label).collect())
        .unwrap_or_default()
}

#[test]
fn adding_groups_and_searching() {
    let (mut section, _, _temp) = setup(Vec::new());
    add(
        &mut section,
        "VS Code",
        ShortcutKind::Program,
        "code",
        "Dev",
    );
    add(
        &mut section,
        "Projetos",
        ShortcutKind::Folder,
        "~/projetos",
        "Pastas",
    );
    add(
        &mut section,
        "GitHub",
        ShortcutKind::Url,
        "github.com",
        "Dev",
    );
    assert!(section.modal().is_none());
    let groups = section.grouped();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, "Dev");
    assert_eq!(groups[0].1.len(), 2);
    assert_eq!(section.shortcuts()[0].target, PerOs::Same("code".into()));
    send(&mut section, [Message::QueryChanged("git".into())]);
    let groups = section.grouped();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1[0].1.name, "GitHub");
}

#[test]
fn invalid_form_shows_error() {
    let (mut section, _, _temp) = setup(Vec::new());
    send(&mut section, [Message::NewShortcut, Message::FormSaved]);
    assert!(section.form_error().is_some());
    assert!(matches!(section.modal(), Some(Modal::Form(_))));
}

#[test]
fn open_edit_and_delete() {
    let (mut section, platform, _temp) = setup(Vec::new());
    add(
        &mut section,
        "Site",
        ShortcutKind::Url,
        "exemplo.com",
        "Web",
    );
    send(&mut section, [Message::Open(0)]);
    assert!(
        platform
            .calls()
            .contains(&PlatformCall::OpenUrl("https://exemplo.com".into()))
    );
    send(
        &mut section,
        [
            Message::Edit(0),
            Message::Form(FormMessage::NameChanged("Site novo".into())),
            Message::FormSaved,
        ],
    );
    assert_eq!(section.shortcuts()[0].name, "Site novo");
    send(&mut section, [Message::AskDelete(0)]);
    assert_eq!(section.shortcuts().len(), 1);
    send(&mut section, [Message::DeleteConfirmed]);
    assert!(section.shortcuts().is_empty());
}

#[test]
fn card_menu_offers_actions_by_kind() {
    let (mut section, _, _temp) = setup(Vec::new());
    add(
        &mut section,
        "Editor",
        ShortcutKind::Program,
        "C:\\editor.exe",
        "G",
    );
    add(&mut section, "Site", ShortcutKind::Url, "a.com", "G");
    send(&mut section, [Message::CardMenu(0)]);
    assert_eq!(
        menu_labels(&section),
        vec!["Abrir", "Editar", "Abrir local do arquivo", "Excluir"]
    );
    let menu = section.context_menu().unwrap();
    assert!(menu.items[3].danger);
    assert!(matches!(menu.items[3].message, Some(Message::AskDelete(0))));
    send(&mut section, [Message::CardMenu(1)]);
    assert_eq!(menu_labels(&section), vec!["Abrir", "Editar", "Excluir"]);
    send(&mut section, [Message::MenuClosed]);
    assert!(section.context_menu().is_none());
}

#[test]
fn card_menu_disables_open_without_target() {
    let (mut section, _, _temp) = setup(Vec::new());
    send(
        &mut section,
        [
            Message::NewShortcut,
            Message::Form(FormMessage::NameChanged("Só Linux".into())),
            Message::Form(FormMessage::LinuxChanged("gedit".into())),
            Message::FormSaved,
            Message::CardMenu(0),
        ],
    );
    let menu = section.context_menu().unwrap();
    assert_eq!(menu.items[0].label, "Abrir");
    assert!(menu.items[0].message.is_none());
    assert!(!menu_labels(&section).contains(&"Abrir local do arquivo".to_string()));
}

#[test]
fn menu_actions_close_menu_and_run() {
    let (mut section, platform, _temp) = setup(Vec::new());
    add(
        &mut section,
        "Projetos",
        ShortcutKind::Folder,
        "C:\\Projetos",
        "G",
    );
    send(&mut section, [Message::CardMenu(0), Message::Reveal(0)]);
    assert!(section.context_menu().is_none());
    assert_eq!(
        platform.calls().last(),
        Some(&PlatformCall::RevealPath(PathBuf::from("C:\\Projetos")))
    );
    send(&mut section, [Message::CardMenu(0), Message::AskDelete(0)]);
    assert!(section.context_menu().is_none());
    assert!(matches!(section.modal(), Some(Modal::ConfirmDelete(0))));
    send(&mut section, [Message::DeleteConfirmed]);
    assert!(section.shortcuts().is_empty());
}

#[test]
fn area_menu_creates_shortcut() {
    let (mut section, _, _temp) = setup(Vec::new());
    send(&mut section, [Message::AreaMenu]);
    assert_eq!(menu_labels(&section), vec!["Novo atalho"]);
    send(&mut section, [Message::NewShortcut]);
    assert!(section.context_menu().is_none());
    assert!(section.form().is_some());
    send(&mut section, [Message::AreaMenu]);
    assert!(section.context_menu().is_none());
}

#[test]
fn form_loads_apps_and_picks_one() {
    let apps = vec![
        app("Firefox", "C:\\firefox.lnk"),
        app("Calculadora Científica", "C:\\calc.lnk"),
        app("Steam", "C:\\steam.lnk"),
    ];
    let (mut section, _, _temp) = setup(apps.clone());
    send(&mut section, [Message::NewShortcut]);
    assert_eq!(section.apps(), &Apps::Loading);
    send(
        &mut section,
        [
            Message::AppsLoaded(Ok(apps)),
            Message::AppQueryChanged("CIENTIF".into()),
        ],
    );
    let visible = section.visible_apps();
    assert_eq!(visible.len(), 1);
    let index = visible[0].0;
    send(
        &mut section,
        [
            Message::AppPicked(index),
            Message::Form(FormMessage::GroupChanged("Programas".into())),
            Message::FormSaved,
        ],
    );
    let saved = &section.shortcuts()[0];
    assert_eq!(saved.name, "Calculadora Científica");
    assert_eq!(saved.kind, ShortcutKind::Program);
    assert_eq!(saved.target.windows(), Some("C:\\calc.lnk"));
    assert_eq!(saved.target.linux(), None);
    send(&mut section, [Message::NewShortcut]);
    assert!(matches!(section.apps(), Apps::Ready(_)));
    assert_eq!(section.app_query(), "");
}

#[test]
fn failed_app_listing_is_retried() {
    let (mut section, _, _temp) = setup(Vec::new());
    send(
        &mut section,
        [
            Message::NewShortcut,
            Message::AppsLoaded(Err("falhou".into())),
        ],
    );
    assert_eq!(section.apps(), &Apps::Failed("falhou".into()));
    send(&mut section, [Message::ModalClosed, Message::NewShortcut]);
    assert_eq!(section.apps(), &Apps::Loading);
}

#[test]
fn picked_paths_fill_target_and_name() {
    let (mut section, _, _temp) = setup(Vec::new());
    send(
        &mut section,
        [
            Message::NewShortcut,
            Message::FilePicked(Some(PathBuf::from("C:\\Apps\\tool.exe"))),
        ],
    );
    let form = section.form().unwrap();
    assert_eq!(form.windows, "C:\\Apps\\tool.exe");
    assert_eq!(form.name, "tool");
    assert_eq!(form.kind, ShortcutKind::Program);
    send(
        &mut section,
        [
            Message::FolderPicked(Some(PathBuf::from("C:\\Projetos"))),
            Message::FilePicked(None),
        ],
    );
    let form = section.form().unwrap();
    assert_eq!(form.windows, "C:\\Projetos");
    assert_eq!(form.name, "tool");
    assert_eq!(form.kind, ShortcutKind::Folder);
}

#[test]
fn icons_are_requested_cached_and_shown() {
    let (mut section, _, _temp) = setup(Vec::new());
    add(
        &mut section,
        "Editor",
        ShortcutKind::Program,
        "C:\\editor.exe",
        "G",
    );
    add(
        &mut section,
        "Pasta",
        ShortcutKind::Folder,
        "C:\\Pasta",
        "G",
    );
    add(&mut section, "Site", ShortcutKind::Url, "a.com", "G");
    assert_eq!(
        section.missing_icons(),
        vec![PathBuf::from("C:\\editor.exe"), PathBuf::from("C:\\Pasta")]
    );
    let handle = icon_handle(FileIcon::Rgba {
        width: 1,
        height: 1,
        pixels: vec![1, 2, 3, 255],
    });
    send(
        &mut section,
        [Message::IconsLoaded(vec![
            (PathBuf::from("C:\\editor.exe"), Some(handle)),
            (PathBuf::from("C:\\Pasta"), None),
        ])],
    );
    assert!(section.missing_icons().is_empty());
    assert!(section.shortcut_icon(&section.shortcuts()[0]).is_some());
    assert!(section.shortcut_icon(&section.shortcuts()[1]).is_none());
    let _ = section.view();
}

#[test]
fn app_icons_are_requested_only_for_program_forms() {
    let apps = vec![app("Firefox", "C:\\firefox.lnk")];
    let (mut section, _, _temp) = setup(apps.clone());
    send(
        &mut section,
        [Message::NewShortcut, Message::AppsLoaded(Ok(apps))],
    );
    assert_eq!(
        section.missing_icons(),
        vec![PathBuf::from("C:\\firefox.lnk")]
    );
    send(
        &mut section,
        [Message::Form(FormMessage::KindSelected(ShortcutKind::Url))],
    );
    assert!(section.missing_icons().is_empty());
}

#[test]
fn ellipsize_keeps_start_and_end() {
    assert_eq!(ellipsize("curto", 10), "curto");
    let long = "C:\\Program Files\\Microsoft VS Code\\Code.exe";
    let short = ellipsize(long, 20);
    assert_eq!(short.chars().count(), 20);
    assert!(short.starts_with("C:\\Pro"));
    assert!(short.ends_with("Code\\Code.exe"));
    assert!(short.contains('…'));
}

#[test]
fn view_builds_in_every_state() {
    let (mut section, _, _temp) = setup(Vec::new());
    let _ = section.view();
    add(&mut section, "A", ShortcutKind::Command, "echo oi", "G");
    let _ = section.view();
    send(&mut section, [Message::NewShortcut]);
    let _ = section.view();
    send(
        &mut section,
        [Message::AppsLoaded(Ok(vec![app("Firefox", "C:\\f.lnk")]))],
    );
    let _ = section.view();
    send(
        &mut section,
        [Message::AppsLoaded(Err("sem acesso".into()))],
    );
    let _ = section.view();
    send(&mut section, [Message::Edit(0)]);
    let _ = section.view();
    send(&mut section, [Message::AskDelete(0)]);
    let _ = section.view();
    send(&mut section, [Message::ModalClosed, Message::CardMenu(0)]);
    let _ = section.view();
}

fn order(section: &Shortcuts) -> Vec<String> {
    section
        .shortcuts()
        .iter()
        .map(|s| format!("{}:{}", s.group, s.name))
        .collect()
}

fn far() -> Point {
    Point::new(10_000.0, 10_000.0)
}

#[test]
fn click_without_moving_opens_shortcut() {
    let (mut section, platform, _temp) = setup(Vec::new());
    add(
        &mut section,
        "Site",
        ShortcutKind::Url,
        "exemplo.com",
        "Web",
    );
    send(
        &mut section,
        [Message::ShortcutPressed(0), Message::DragReleased],
    );
    assert!(
        platform
            .calls()
            .contains(&PlatformCall::OpenUrl("https://exemplo.com".into()))
    );
    assert!(section.drag().is_none());
}

#[test]
fn dragging_shortcuts_reorders_and_changes_group() {
    let (mut section, platform, _temp) = setup(Vec::new());
    add(&mut section, "A", ShortcutKind::Url, "a.com", "G1");
    add(&mut section, "B", ShortcutKind::Url, "b.com", "G1");
    add(&mut section, "X", ShortcutKind::Url, "x.com", "G2");
    send(
        &mut section,
        [
            Message::ShortcutPressed(0),
            Message::DragMoved(far()),
            Message::GroupHovered("G1".into()),
            Message::TileHovered(1),
        ],
    );
    assert_eq!(section.dragging(), Some(&DragItem::Shortcut(0)));
    let _ = section.view();
    send(&mut section, [Message::DragReleased]);
    assert_eq!(order(&section), ["G1:B", "G1:A", "G2:X"]);
    send(
        &mut section,
        [
            Message::TileLeft(1),
            Message::GroupLeft("G1".into()),
            Message::ShortcutPressed(0),
            Message::DragMoved(far()),
            Message::GroupHovered("G2".into()),
            Message::DragReleased,
        ],
    );
    assert_eq!(order(&section), ["G1:A", "G2:X", "G2:B"]);
    assert!(platform.calls().is_empty());
}

#[test]
fn dragging_group_header_reorders_groups() {
    let (mut section, _platform, _temp) = setup(Vec::new());
    add(&mut section, "A", ShortcutKind::Url, "a.com", "G1");
    add(&mut section, "X", ShortcutKind::Url, "x.com", "G2");
    send(
        &mut section,
        [
            Message::GroupPressed("G2".into()),
            Message::DragMoved(far()),
            Message::GroupHovered("G1".into()),
        ],
    );
    let _ = section.view();
    send(&mut section, [Message::DragReleased]);
    assert_eq!(order(&section), ["G2:X", "G1:A"]);
}

#[test]
fn clicking_group_header_collapses_and_persists() {
    let (mut section, _platform, temp) = setup(Vec::new());
    add(&mut section, "A", ShortcutKind::Url, "a.com", "G1");
    send(
        &mut section,
        [Message::GroupPressed("G1".into()), Message::DragReleased],
    );
    assert!(section.is_collapsed("G1"));
    let _ = section.view();
    let saved = std::fs::read_to_string(HubPaths::from_home(temp.path()).shortcuts_file()).unwrap();
    assert!(saved.contains("recolhidos"));
    send(&mut section, [Message::QueryChanged("a".into())]);
    assert!(!section.is_collapsed("G1"));
    send(
        &mut section,
        [
            Message::QueryChanged(String::new()),
            Message::GroupToggled("G1".into()),
        ],
    );
    assert!(!section.is_collapsed("G1"));
}
