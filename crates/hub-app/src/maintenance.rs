use std::path::PathBuf;

use hub_core::HubContext;
use hub_core::backup::{self, BackupInfo, BackupReason};
use hub_core::channel::{self, Channel};
use hub_core::platform::Notification;
use hub_core::update::{self, UpdateInfo};
use hub_ui_kit::dialog;
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use iced::widget::{Column, column, row, space};
use iced::{Alignment, Element, Task};

#[derive(Debug, Clone)]
pub enum Message {
    CheckUpdate {
        silent: bool,
    },
    UpdateChecked {
        silent: bool,
        result: Result<Option<UpdateInfo>, String>,
    },
    InstallUpdate,
    UpdateLaunched(Result<(), String>),
    BackupNow,
    BackupDone(Result<BackupInfo, String>),
    DailyDone(Result<Option<BackupInfo>, String>),
    PickRestore,
    RestoreChosen(Option<PathBuf>),
    OpenBackups,
    Restart,
}

#[derive(Debug, Clone, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Installing,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct Maintenance {
    pub update: UpdateState,
    pub backups: Vec<BackupInfo>,
    pub busy: bool,
    pub status: Option<String>,
    pub restart_pending: bool,
}

pub enum Outcome {
    None,
    Task(Task<Message>),
    Quit,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

impl Maintenance {
    pub fn new(ctx: &HubContext) -> Self {
        Self {
            backups: backup::list(&ctx.paths),
            ..Self::default()
        }
    }

    pub fn boot_tasks(ctx: &HubContext) -> Task<Message> {
        let mut tasks = vec![daily_task(ctx)];
        let config = ctx.config();
        if Channel::current() == Channel::Release && config.update.check_on_start {
            tasks.push(Task::done(Message::CheckUpdate { silent: true }));
        }
        Task::batch(tasks)
    }

    pub fn daily(ctx: &HubContext) -> Task<Message> {
        daily_task(ctx)
    }

    pub fn update(&mut self, ctx: &HubContext, message: Message) -> Outcome {
        match message {
            Message::CheckUpdate { silent } => {
                if Channel::current() != Channel::Release {
                    self.update = UpdateState::Failed(
                        "Atualizações só existem na versão instalada".to_string(),
                    );
                    return Outcome::None;
                }
                self.update = UpdateState::Checking;
                let folder = ctx.config().update.update_folder();
                return Outcome::Task(Task::perform(
                    blocking(move || {
                        update::check(&folder, channel::VERSION).map_err(|e| e.to_string())
                    }),
                    move |result| Message::UpdateChecked { silent, result },
                ));
            }
            Message::UpdateChecked { silent, result } => {
                self.update = match result {
                    Ok(Some(info)) => {
                        if silent {
                            notify_available(ctx, &info);
                        }
                        UpdateState::Available(info)
                    }
                    Ok(None) => UpdateState::UpToDate,
                    Err(error) if silent => {
                        tracing::warn!(%error, "falha ao procurar atualização");
                        UpdateState::Idle
                    }
                    Err(error) => UpdateState::Failed(error),
                };
            }
            Message::InstallUpdate => {
                let UpdateState::Available(info) = &self.update else {
                    return Outcome::None;
                };
                let info = info.clone();
                let paths = ctx.paths.clone();
                let now = ctx.clock.now();
                self.update = UpdateState::Installing;
                return Outcome::Task(Task::perform(
                    blocking(move || update::apply(&paths, &info, now).map_err(|e| e.to_string())),
                    Message::UpdateLaunched,
                ));
            }
            Message::UpdateLaunched(Ok(())) => {
                tracing::info!("instalador iniciado, encerrando para atualizar");
                return Outcome::Quit;
            }
            Message::UpdateLaunched(Err(error)) => {
                self.update = UpdateState::Failed(error);
            }
            Message::BackupNow => {
                self.busy = true;
                self.status = Some("Fazendo backup".to_string());
                let paths = ctx.paths.clone();
                let now = ctx.clock.now();
                return Outcome::Task(Task::perform(
                    blocking(move || {
                        backup::create(&paths, BackupReason::Manual, now).map_err(|e| e.to_string())
                    }),
                    Message::BackupDone,
                ));
            }
            Message::BackupDone(result) => {
                self.busy = false;
                self.status = Some(match result {
                    Ok(info) => format!("Backup criado: {}", file_name(&info.path)),
                    Err(error) => format!("Falha no backup: {error}"),
                });
                self.backups = backup::list(&ctx.paths);
            }
            Message::DailyDone(result) => {
                match result {
                    Ok(Some(info)) => {
                        tracing::info!(path = %info.path.display(), "backup diário criado")
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(%error, "falha no backup diário"),
                }
                self.backups = backup::list(&ctx.paths);
            }
            Message::PickRestore => {
                return Outcome::Task(Task::perform(
                    dialog::pick_file("Escolher backup para restaurar"),
                    Message::RestoreChosen,
                ));
            }
            Message::RestoreChosen(None) => {}
            Message::RestoreChosen(Some(path)) => {
                match backup::schedule_restore(&ctx.paths, &path) {
                    Ok(()) => {
                        self.restart_pending = true;
                        self.status = Some(format!(
                            "{} será restaurado ao reiniciar o hub",
                            file_name(&path)
                        ));
                    }
                    Err(error) => self.status = Some(format!("Backup inválido: {error}")),
                }
            }
            Message::OpenBackups => {
                let dir = ctx.paths.backups_dir();
                let _ = std::fs::create_dir_all(&dir);
                if let Err(error) = ctx.platform.open_path(&dir) {
                    self.status = Some(format!("Não foi possível abrir a pasta: {error}"));
                }
            }
            Message::Restart => match std::env::current_exe()
                .and_then(|exe| std::process::Command::new(exe).spawn())
            {
                Ok(_) => return Outcome::Quit,
                Err(error) => self.status = Some(format!("Não foi possível reiniciar: {error}")),
            },
        }
        Outcome::None
    }

    pub fn view(&self) -> Element<'_, Message> {
        let release = Channel::current() == Channel::Release;
        let update_line: Element<'_, Message> = if !release {
            muted("A versão dev não se atualiza; publique com scripts/publicar.ps1").into()
        } else {
            match &self.update {
                UpdateState::Idle => row![
                    muted("Nenhuma verificação feita"),
                    ghost_button(
                        "Procurar atualização",
                        Some(Message::CheckUpdate { silent: false })
                    ),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center)
                .into(),
                UpdateState::Checking => muted("Procurando atualização").into(),
                UpdateState::UpToDate => row![
                    body("Você está na versão mais recente"),
                    ghost_button(
                        "Procurar de novo",
                        Some(Message::CheckUpdate { silent: false })
                    ),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center)
                .into(),
                UpdateState::Available(info) => {
                    let mut lines = Column::new().spacing(units(1));
                    lines = lines.push(body(format!("Versão {} disponível", info.version)));
                    if !info.notes.trim().is_empty() {
                        lines = lines.push(muted(info.notes.as_str()));
                    }
                    row![
                        lines,
                        primary_button("Instalar e reiniciar", Some(Message::InstallUpdate)),
                    ]
                    .spacing(units(3))
                    .align_y(Alignment::Center)
                    .into()
                }
                UpdateState::Installing => muted("Fazendo backup e iniciando o instalador").into(),
                UpdateState::Failed(error) => row![
                    body(error.as_str()).color(danger()),
                    ghost_button(
                        "Tentar de novo",
                        Some(Message::CheckUpdate { silent: false })
                    ),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center)
                .into(),
            }
        };

        let mut actions = row![
            ghost_button(
                "Fazer backup agora",
                (!self.busy).then_some(Message::BackupNow)
            ),
            ghost_button("Restaurar backup", Some(Message::PickRestore)),
            ghost_button("Abrir pasta de backups", Some(Message::OpenBackups)),
        ]
        .spacing(units(2));
        if self.restart_pending {
            actions = actions.push(primary_button("Reiniciar agora", Some(Message::Restart)));
        }

        let mut list = Column::new().spacing(units(1));
        for info in self.backups.iter().take(5) {
            list = list.push(
                row![
                    body(info.created.format("%d/%m/%Y %H:%M").to_string()).width(140),
                    muted(info.reason.label()).width(140),
                    muted(format_size(info.size)),
                ]
                .spacing(units(3)),
            );
        }
        if self.backups.is_empty() {
            list = list.push(muted("Nenhum backup ainda"));
        }

        let mut content = column![
            heading("Atualização"),
            update_line,
            space().height(units(2)),
            heading("Backups"),
            muted(
                "Um backup diário automático é feito ao abrir o hub e antes de cada atualização."
            ),
            actions,
        ]
        .spacing(units(2));
        if let Some(status) = &self.status {
            content = content.push(muted(status.as_str()));
        }
        content.push(list).into()
    }
}

fn daily_task(ctx: &HubContext) -> Task<Message> {
    let config = ctx.config().backup;
    if !config.automatic {
        return Task::none();
    }
    let paths = ctx.paths.clone();
    let now = ctx.clock.now();
    let keep = config.keep as usize;
    Task::perform(
        blocking(move || {
            if !backup::needs_daily(&paths, now.date()) {
                return Ok(None);
            }
            let info =
                backup::create(&paths, BackupReason::Daily, now).map_err(|e| e.to_string())?;
            backup::prune(&paths, keep);
            Ok(Some(info))
        }),
        Message::DailyDone,
    )
}

fn notify_available(ctx: &HubContext, info: &UpdateInfo) {
    let notification = Notification {
        title: format!("{} {}", channel::app_name(), info.version),
        body: "Nova versão disponível. Abra as configurações para instalar.".to_string(),
        actions: Vec::new(),
    };
    ctx.notify(notification, None);
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn format_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{} KB", bytes / 1024)
    }
}
