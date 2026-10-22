use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::{FakePlatform, PlatformCall};
use hub_core::scripts::NewScript;
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

fn setup() -> (Scripts, HubContext, Arc<FakePlatform>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    let platform = Arc::new(FakePlatform::default());
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform.clone(),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    (Scripts::new(ctx.clone()), ctx, platform, temp)
}

fn send(section: &mut Scripts, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = section.update(message);
    }
}

fn create_script(ctx: &HubContext, name: &str, confirm: bool) {
    scripts::create(
        &ctx.paths.scripts_dir,
        NewScript {
            name: name.into(),
            windows_file: Some(format!("{}.ps1", scripts::slug(name))),
            linux_file: Some(format!("{}.sh", scripts::slug(name))),
            confirm,
            active: true,
            ..NewScript::default()
        },
    )
    .unwrap();
}

#[test]
fn form_creates_script_and_selects_it() {
    let (mut section, _, _, _temp) = setup();
    send(
        &mut section,
        [
            Message::NewScript,
            Message::Form(FormMessage::NameChanged("Limpar temp".into())),
            Message::Form(FormMessage::SchedulePreset("0 0 3 * * *")),
            Message::FormSaved,
        ],
    );
    assert!(section.modal().is_none());
    assert_eq!(section.scripts().len(), 1);
    assert_eq!(section.selected().unwrap().id, "limpar-temp");
    assert_eq!(
        section.scripts()[0].meta.schedule.as_deref(),
        Some("0 0 3 * * *")
    );
}

#[test]
fn invalid_schedule_keeps_form_open() {
    let (mut section, _, _, _temp) = setup();
    send(
        &mut section,
        [
            Message::NewScript,
            Message::Form(FormMessage::NameChanged("x".into())),
            Message::Form(FormMessage::ScheduleChanged("sempre".into())),
            Message::FormSaved,
        ],
    );
    assert!(matches!(section.modal(), Some(Modal::Form(_))));
    assert!(section.form_error().is_some());
    assert!(section.scripts().is_empty());
}

#[test]
fn edit_updates_metadata() {
    let (mut section, ctx, _, _temp) = setup();
    create_script(&ctx, "Backup", false);
    send(
        &mut section,
        [
            Message::Refresh,
            Message::EditScript("backup".into()),
            Message::Form(FormMessage::DescriptionChanged("copia tudo".into())),
            Message::Form(FormMessage::ConfirmToggled(true)),
            Message::FormSaved,
        ],
    );
    let script = section.script("backup").unwrap();
    assert_eq!(script.meta.description, "copia tudo");
    assert!(script.meta.confirm);
}

#[test]
fn run_starts_immediately_or_asks_confirmation() {
    let (mut section, ctx, _, _temp) = setup();
    create_script(&ctx, "Rapido", false);
    create_script(&ctx, "Cuidado", true);
    send(&mut section, [Message::Refresh]);
    send(&mut section, [Message::RunRequested("rapido".into())]);
    assert!(section.run("rapido").unwrap().is_running());
    assert!(section.run("rapido").unwrap().can_cancel());
    send(&mut section, [Message::RunRequested("cuidado".into())]);
    assert!(section.run("cuidado").is_none());
    assert!(matches!(section.modal(), Some(Modal::ConfirmRun(_))));
    send(&mut section, [Message::RunConfirmed]);
    assert!(section.run("cuidado").unwrap().is_running());
}

#[test]
fn events_fill_console_and_finish() {
    let (mut section, ctx, _, _temp) = setup();
    create_script(&ctx, "Backup", false);
    send(
        &mut section,
        [
            Message::Refresh,
            Message::External(
                "backup".into(),
                Origin::Scheduled,
                RunEvent::Started { execution_id: 7 },
            ),
            Message::External(
                "backup".into(),
                Origin::Scheduled,
                RunEvent::Line {
                    stream: OutputStream::Stdout,
                    text: "ok".into(),
                },
            ),
        ],
    );
    let run = section.run("backup").unwrap();
    assert_eq!(run.origin, Origin::Scheduled);
    assert!(!run.can_cancel());
    assert_eq!(run.lines[0].text, "ok");
    send(
        &mut section,
        [Message::External(
            "backup".into(),
            Origin::Scheduled,
            RunEvent::Finished {
                execution_id: 7,
                status: Status::Completed(0),
                output: "ok\n".into(),
            },
        )],
    );
    assert!(!section.run("backup").unwrap().is_running());
}

#[test]
fn script_without_platform_file_reports_problem() {
    let (mut section, ctx, _, _temp) = setup();
    scripts::create(
        &ctx.paths.scripts_dir,
        NewScript {
            name: "Outro sistema".into(),
            windows_file: None,
            linux_file: None,
            ..NewScript::default()
        },
    )
    .unwrap_err();
    std::fs::write(
        ctx.paths.scripts_dir.join("orfao.toml"),
        "nome = \"Órfão\"\narquivo = { windows = \"nao-existe.ps1\", linux = \"nao-existe.sh\" }\n",
    )
    .unwrap();
    send(
        &mut section,
        [Message::Refresh, Message::RunRequested("orfao".into())],
    );
    assert!(section.run("orfao").is_none());
    assert!(section.error().is_some());
}

#[test]
fn toggling_active_and_deleting() {
    let (mut section, ctx, _, _temp) = setup();
    create_script(&ctx, "Backup", false);
    send(
        &mut section,
        [
            Message::Refresh,
            Message::ActiveToggled("backup".into(), false),
        ],
    );
    assert!(!section.script("backup").unwrap().meta.active);
    send(&mut section, [Message::AskDelete("backup".into())]);
    assert!(section.script("backup").is_some());
    send(&mut section, [Message::DeleteConfirmed]);
    assert!(section.script("backup").is_none());
}

#[test]
fn open_file_uses_platform() {
    let (mut section, ctx, platform, _temp) = setup();
    create_script(&ctx, "Backup", false);
    send(
        &mut section,
        [
            Message::Refresh,
            Message::OpenFile("backup".into()),
            Message::OpenFolder,
        ],
    );
    let calls = platform.calls();
    assert!(
        matches!(&calls[0], PlatformCall::OpenPath(p) if p.ends_with("backup.ps1") || p.ends_with("backup.sh"))
    );
    assert!(matches!(&calls[1], PlatformCall::OpenPath(p) if p.ends_with("scripts")));
}

#[test]
fn view_builds_in_every_state() {
    let (mut section, ctx, _, _temp) = setup();
    let _ = section.view();
    create_script(&ctx, "Backup", true);
    send(&mut section, [Message::Refresh]);
    let _ = section.view();
    send(&mut section, [Message::RunRequested("backup".into())]);
    let _ = section.view();
    send(&mut section, [Message::RunConfirmed]);
    let _ = section.view();
    send(&mut section, [Message::NewScript]);
    let _ = section.view();
    send(
        &mut section,
        [Message::ModalClosed, Message::AskDelete("backup".into())],
    );
    let _ = section.view();
}
