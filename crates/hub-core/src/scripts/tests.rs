use std::sync::Arc;

use chrono::NaiveDateTime;
use futures::StreamExt;

use super::runner::{RunEvent, Status};
use super::*;
use crate::clock::FakeClock;
use crate::platform::FakePlatform;
use crate::{Config, Database, HubContext, HubPaths};

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
}

fn new_script(name: &str) -> NewScript {
    NewScript {
        name: name.into(),
        description: "teste".into(),
        windows_file: Some(format!("{}.ps1", slug(name))),
        linux_file: Some(format!("{}.sh", slug(name))),
        windows_content: "Write-Output oi\n".into(),
        linux_content: "echo oi\n".into(),
        active: true,
        ..NewScript::default()
    }
}

#[test]
fn slug_removes_accents_and_symbols() {
    assert_eq!(slug("Limpar temporários!"), "limpar-temporarios");
    assert_eq!(slug("  Backup  do   PC "), "backup-do-pc");
    assert_eq!(slug("!!!"), "");
}

#[test]
fn schedules_accept_five_or_six_fields() {
    assert!(parse_schedule("0 0 3 * * *").is_ok());
    assert!(parse_schedule("30 8 * * *").is_ok());
    assert!(parse_schedule("todo dia").is_err());
    assert_eq!(
        next_run("0 0 3 * * *", at("2026-10-21 10:00")),
        Some(at("2026-10-22 03:00"))
    );
    assert_eq!(
        next_run("*/15 * * * *", at("2026-10-21 10:01")),
        Some(at("2026-10-21 10:15"))
    );
}

#[test]
fn create_writes_meta_and_code_files() {
    let temp = tempfile::tempdir().unwrap();
    let id = create(temp.path(), new_script("Limpar temp")).unwrap();
    assert_eq!(id, "limpar-temp");
    assert!(temp.path().join("limpar-temp.toml").is_file());
    assert!(temp.path().join("limpar-temp.ps1").is_file());
    assert!(temp.path().join("limpar-temp.sh").is_file());
    assert!(matches!(
        create(temp.path(), new_script("Limpar temp")),
        Err(ScriptError::AlreadyExists(_))
    ));
    let text = std::fs::read_to_string(temp.path().join("limpar-temp.toml")).unwrap();
    assert!(text.contains("nome = \"Limpar temp\""));
    assert!(text.contains("ativo = true"));
}

#[test]
fn create_validates_input() {
    let temp = tempfile::tempdir().unwrap();
    let mut no_file = new_script("x");
    no_file.windows_file = None;
    no_file.linux_file = Some(" ".into());
    assert!(matches!(
        create(temp.path(), no_file),
        Err(ScriptError::NoFile)
    ));
    let mut bad_schedule = new_script("y");
    bad_schedule.schedule = Some("sempre".into());
    assert!(matches!(
        create(temp.path(), bad_schedule),
        Err(ScriptError::InvalidSchedule(_))
    ));
    assert!(matches!(
        create(temp.path(), new_script("  ")),
        Err(ScriptError::EmptyName)
    ));
}

#[test]
fn list_reports_missing_platform_file() {
    let temp = tempfile::tempdir().unwrap();
    let mut only_linux = new_script("Só Linux");
    only_linux.windows_file = None;
    create(temp.path(), only_linux).unwrap();
    let mut store = ScriptStore::default();
    let windows = store.list(temp.path(), Os::Windows);
    assert!(!windows[0].can_run());
    assert!(
        windows[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("Sem versão")
    );
    let linux = store.list(temp.path(), Os::Linux);
    assert!(linux[0].can_run());
}

#[test]
fn invalid_toml_keeps_last_valid_version() {
    let temp = tempfile::tempdir().unwrap();
    create(temp.path(), new_script("Backup")).unwrap();
    let mut store = ScriptStore::default();
    assert_eq!(store.list(temp.path(), Os::Windows)[0].meta.name, "Backup");
    std::fs::write(temp.path().join("backup.toml"), "nome = ").unwrap();
    let scripts = store.list(temp.path(), Os::Windows);
    assert_eq!(scripts[0].meta.name, "Backup");
    assert!(
        scripts[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("última versão válida")
    );
    assert!(!scripts[0].can_run());
    let mut fresh = ScriptStore::default();
    assert!(fresh.list(temp.path(), Os::Windows)[0].problem.is_some());
}

#[test]
fn save_meta_and_delete() {
    let temp = tempfile::tempdir().unwrap();
    create(temp.path(), new_script("Backup")).unwrap();
    let mut store = ScriptStore::default();
    let script = store.get(temp.path(), Os::Windows, "backup").unwrap();
    let mut meta = script.meta.clone();
    meta.schedule = Some("0 0 3 * * *".into());
    meta.active = false;
    save_meta(&script, &meta).unwrap();
    let reloaded = store.get(temp.path(), Os::Windows, "backup").unwrap();
    assert_eq!(reloaded.meta.schedule.as_deref(), Some("0 0 3 * * *"));
    assert!(!reloaded.meta.active);
    meta.schedule = Some("nunca".into());
    assert!(save_meta(&reloaded, &meta).is_err());
    delete(temp.path(), &reloaded).unwrap();
    assert!(store.list(temp.path(), Os::Windows).is_empty());
    assert!(!temp.path().join("backup.ps1").exists());
}

#[test]
fn due_and_next_only_consider_active_runnable_scripts() {
    let temp = tempfile::tempdir().unwrap();
    let mut daily = new_script("Diário");
    daily.schedule = Some("0 0 3 * * *".into());
    create(temp.path(), daily).unwrap();
    let mut inactive = new_script("Inativo");
    inactive.schedule = Some("0 0 1 * * *".into());
    inactive.active = false;
    create(temp.path(), inactive).unwrap();
    let scripts = ScriptStore::default().list(temp.path(), Os::Windows);
    assert_eq!(
        next_scheduled(&scripts, at("2026-10-21 10:00")),
        Some(at("2026-10-22 03:00"))
    );
    assert!(due_between(&scripts, at("2026-10-21 10:00"), at("2026-10-22 02:59")).is_empty());
    let due = due_between(&scripts, at("2026-10-22 02:59"), at("2026-10-22 03:00"));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, "diario");
}

#[test]
fn executions_are_recorded_and_truncated() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let id = start_execution(c, "backup", Origin::Ai, at("2026-10-21 10:00")).unwrap();
    let big = "x".repeat(OUTPUT_LIMIT + 10);
    finish_execution(c, id, Some(0), &big, at("2026-10-21 10:01")).unwrap();
    let list = executions(c, "backup", 10).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].origin, Origin::Ai);
    assert!(list[0].succeeded());
    assert_eq!(list[0].output.len(), OUTPUT_LIMIT);
    let last = last_executions(c).unwrap();
    assert_eq!(last["backup"].id, id);
}

fn runner_setup(
    body_windows: &str,
    body_linux: &str,
    timeout: u64,
) -> (HubContext, Script, Arc<FakePlatform>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    let mut script = new_script("Teste");
    script.windows_content = body_windows.into();
    script.linux_content = body_linux.into();
    script.timeout_secs = timeout;
    create(&paths.scripts_dir, script).unwrap();
    let platform = Arc::new(FakePlatform::default());
    let ctx = HubContext::new(
        paths.clone(),
        Config::default(),
        Database::open_in_memory().unwrap(),
        platform.clone(),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    let script = ScriptStore::default()
        .get(&paths.scripts_dir, Os::current(), "teste")
        .unwrap();
    (ctx, script, platform, temp)
}

#[tokio::test]
async fn runner_streams_output_and_records_exit_code() {
    let (ctx, script, _, _temp) = runner_setup(
        "Write-Output 'linha 1'\n[Console]::Error.WriteLine('falhou')\nexit 3\n",
        "echo 'linha 1'\necho falhou 1>&2\nexit 3\n",
        30,
    );
    let (events, _cancel) = runner::run(ctx.clone(), script, Origin::Manual);
    let events: Vec<RunEvent> = events.collect().await;
    assert!(matches!(events.first(), Some(RunEvent::Started { .. })));
    let lines: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            RunEvent::Line { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(lines.contains(&"linha 1"));
    assert!(lines.contains(&"falhou"));
    let Some(RunEvent::Finished { status, output, .. }) = events.last() else {
        panic!("faltou o fim")
    };
    assert_eq!(*status, Status::Completed(3));
    assert!(output.contains("linha 1"));
    let history = executions(ctx.db().conn(), "teste", 5).unwrap();
    assert_eq!(history[0].exit_code, Some(3));
    assert!(history[0].finished_at.is_some());
}

#[tokio::test]
async fn runner_can_be_cancelled() {
    let (ctx, script, _, _temp) = runner_setup(
        "Write-Output 'comecou'\nStart-Sleep -Seconds 30\n",
        "echo comecou\nsleep 30\n",
        60,
    );
    let (events, cancel) = runner::run(ctx.clone(), script, Origin::Manual);
    let mut events = Box::pin(events);
    while let Some(event) = events.next().await {
        if matches!(event, RunEvent::Line { .. }) {
            cancel.cancel();
        }
        if let RunEvent::Finished { status, .. } = event {
            assert_eq!(status, Status::Cancelled);
            let history = executions(ctx.db().conn(), "teste", 5).unwrap();
            assert_eq!(history[0].exit_code, None);
            assert!(history[0].output.contains("Cancelado"));
            return;
        }
    }
    panic!("o script não terminou");
}

#[tokio::test]
async fn scheduled_timeout_notifies() {
    let (ctx, script, platform, _temp) = runner_setup("Start-Sleep -Seconds 30\n", "sleep 30\n", 1);
    let (status, _) = runner::run_to_end(ctx, script, Origin::Scheduled).await;
    assert_eq!(status, Status::TimedOut);
    assert_eq!(platform.notifications().len(), 1);
    assert!(platform.notifications()[0].title.contains("Teste"));
}
