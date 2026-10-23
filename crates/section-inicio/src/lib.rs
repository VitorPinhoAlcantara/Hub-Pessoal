mod view;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hub_core::HubContext;
use hub_core::agenda::{self, Reminder, Task};
use hub_core::chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use hub_core::notes::{self, Note};
use hub_core::platform::{FileIcon, Platform};
use hub_core::shortcuts::{self, Shortcut};
use hub_ui_kit::Section;
use hub_ui_kit::iced::futures::{Stream, stream};
use hub_ui_kit::iced::widget::image::Handle;
use hub_ui_kit::iced::{Element, Subscription, Task as IcedTask, time};

pub const AGENDA_LIMIT: usize = 6;
pub const MOST_USED_LIMIT: usize = 15;
pub const NOTE_LINES: usize = 8;
const ICON_BATCH: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub enum Destination {
    Agenda,
    Shortcuts,
    Notes(Option<i64>),
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Navigate(Destination),
    Open(usize),
    IconsLoaded(Vec<(PathBuf, Option<Handle>)>),
    ErrorDismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Appointment,
    Routine,
    Reminder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaItem {
    pub label: String,
    pub title: String,
    pub kind: ItemKind,
    pub overdue: bool,
}

pub struct Home {
    ctx: HubContext,
    now: NaiveDateTime,
    items: Vec<AgendaItem>,
    overdue: usize,
    most_used: Vec<Shortcut>,
    icons: HashMap<PathBuf, Option<Handle>>,
    favorite: Option<Note>,
    error: Option<String>,
}

pub fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "segunda feira",
        Weekday::Tue => "terça feira",
        Weekday::Wed => "quarta feira",
        Weekday::Thu => "quinta feira",
        Weekday::Fri => "sexta feira",
        Weekday::Sat => "sábado",
        Weekday::Sun => "domingo",
    }
}

pub fn month_name(month: u32) -> &'static str {
    match month {
        1 => "janeiro",
        2 => "fevereiro",
        3 => "março",
        4 => "abril",
        5 => "maio",
        6 => "junho",
        7 => "julho",
        8 => "agosto",
        9 => "setembro",
        10 => "outubro",
        11 => "novembro",
        _ => "dezembro",
    }
}

pub fn format_date(date: NaiveDate) -> String {
    format!(
        "{}, {} de {} de {}",
        weekday_name(date.weekday()),
        date.day(),
        month_name(date.month()),
        date.year()
    )
}

pub fn icon_handle(icon: FileIcon) -> Handle {
    match icon {
        FileIcon::Rgba {
            width,
            height,
            pixels,
        } => Handle::from_rgba(width, height, pixels),
        FileIcon::Encoded(bytes) => Handle::from_bytes(bytes),
    }
}

fn task_kind(task: &Task) -> ItemKind {
    if task.routine_id.is_some() {
        ItemKind::Routine
    } else {
        ItemKind::Appointment
    }
}

fn time_label(time: Option<NaiveTime>) -> String {
    match time {
        Some(time) => time.format("%H:%M").to_string(),
        None => "Sem hora".to_string(),
    }
}

impl Home {
    pub fn new(ctx: HubContext) -> Self {
        let now = ctx.clock.now();
        let mut section = Self {
            ctx,
            now,
            items: Vec::new(),
            overdue: 0,
            most_used: Vec::new(),
            icons: HashMap::new(),
            favorite: None,
            error: None,
        };
        section.refresh();
        section
    }

    pub fn refresh(&mut self) {
        self.now = self.ctx.clock.now();
        self.load_agenda();
        self.load_shortcuts();
        self.load_favorite();
    }

    pub fn now(&self) -> NaiveDateTime {
        self.now
    }

    pub fn items(&self) -> &[AgendaItem] {
        &self.items
    }

    pub fn overdue(&self) -> usize {
        self.overdue
    }

    pub fn most_used(&self) -> &[Shortcut] {
        &self.most_used
    }

    pub fn favorite(&self) -> Option<&Note> {
        self.favorite.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn icon(&self, path: &Path) -> Option<&Handle> {
        self.icons.get(path).and_then(Option::as_ref)
    }

    pub fn shortcut_icon(&self, shortcut: &Shortcut) -> Option<&Handle> {
        shortcuts::local_path(shortcut, self.ctx.platform.os()).and_then(|path| self.icon(&path))
    }

    pub fn missing_icons(&self) -> Vec<PathBuf> {
        let os = self.ctx.platform.os();
        let mut missing = Vec::new();
        for path in self
            .most_used
            .iter()
            .filter_map(|shortcut| shortcuts::local_path(shortcut, os))
        {
            if !self.icons.contains_key(&path) && !missing.contains(&path) {
                missing.push(path);
                if missing.len() == ICON_BATCH {
                    break;
                }
            }
        }
        missing
    }

    fn load_agenda(&mut self) {
        let today = self.now.date();
        let (view, reminders) = {
            let db = self.ctx.db();
            let view = agenda::today_view(db.conn(), today).unwrap_or_else(|error| {
                tracing::warn!(%error, "falha ao carregar as tarefas de hoje");
                Default::default()
            });
            let reminders = agenda::loose_reminders_between(db.conn(), today, today)
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "falha ao carregar os lembretes de hoje");
                    Vec::new()
                });
            (view, reminders)
        };
        let mut due: Vec<(Option<NaiveTime>, String, ItemKind)> = view
            .today
            .iter()
            .map(|task| (task.time, task.title.clone(), task_kind(task)))
            .collect();
        due.extend(
            reminders
                .iter()
                .filter(|reminder: &&Reminder| reminder.fired_at.is_none())
                .map(|reminder| {
                    (
                        Some(reminder.effective_time().time()),
                        reminder.title.clone(),
                        ItemKind::Reminder,
                    )
                }),
        );
        due.sort_by_key(|(time, _, _)| (time.is_none(), *time));
        let mut items: Vec<AgendaItem> = due
            .into_iter()
            .map(|(time, title, kind)| AgendaItem {
                label: time_label(time),
                title,
                kind,
                overdue: false,
            })
            .collect();
        items.extend(view.overdue.iter().map(|task| {
            AgendaItem {
                label: task
                    .date
                    .map(|date| date.format("%d/%m").to_string())
                    .unwrap_or_default(),
                title: task.title.clone(),
                kind: task_kind(task),
                overdue: true,
            }
        }));
        items.truncate(AGENDA_LIMIT);
        self.items = items;
        self.overdue = view.overdue.len();
    }

    fn load_shortcuts(&mut self) {
        let loaded = self.ctx.shortcuts();
        let usage = shortcuts::usage(self.ctx.db().conn()).unwrap_or_else(|error| {
            tracing::warn!(%error, "falha ao carregar o uso dos atalhos");
            Default::default()
        });
        self.most_used = shortcuts::most_used(
            &loaded.shortcuts,
            &usage,
            self.ctx.platform.os(),
            MOST_USED_LIMIT,
        );
    }

    fn load_favorite(&mut self) {
        self.favorite = notes::favorite(self.ctx.db().conn()).unwrap_or_else(|error| {
            tracing::warn!(%error, "falha ao carregar a nota favorita");
            None
        });
    }

    fn open(&mut self, index: usize) {
        let Some(shortcut) = self.most_used.get(index).cloned() else {
            return;
        };
        match shortcuts::open(self.ctx.platform.as_ref(), &shortcut) {
            Ok(()) => {
                let key = shortcuts::usage_key(&shortcut, self.ctx.platform.os());
                let recorded =
                    shortcuts::record_use(self.ctx.db().conn(), &key, self.ctx.clock.now());
                if let Err(error) = recorded {
                    tracing::warn!(%error, "falha ao registrar o uso do atalho");
                }
                self.load_shortcuts();
            }
            Err(error) => {
                self.error = Some(format!("Não foi possível abrir {}: {error}", shortcut.name));
            }
        }
    }
}

struct IconJob {
    paths: Vec<PathBuf>,
    platform: Arc<dyn Platform>,
}

impl Hash for IconJob {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.paths.hash(state);
    }
}

fn load_icons(job: &IconJob) -> impl Stream<Item = Message> + use<> {
    let paths = job.paths.clone();
    let platform = job.platform.clone();
    stream::once(async move {
        let loaded = tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .map(|path| {
                    let icon = platform.file_icon(&path).map(icon_handle);
                    (path, icon)
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        Message::IconsLoaded(loaded)
    })
}

impl Section for Home {
    type Message = Message;

    fn title(&self) -> &str {
        "Início"
    }

    fn update(&mut self, message: Message) -> IcedTask<Message> {
        match message {
            Message::Tick => {
                let previous = self.now.date();
                self.now = self.ctx.clock.now();
                if self.now.date() != previous {
                    self.load_agenda();
                }
            }
            Message::Navigate(_) => {}
            Message::Open(index) => self.open(index),
            Message::IconsLoaded(icons) => self.icons.extend(icons),
            Message::ErrorDismissed => self.error = None,
        }
        IcedTask::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![time::every(Duration::from_secs(1)).map(|_| Message::Tick)];
        let missing = self.missing_icons();
        if !missing.is_empty() {
            let job = IconJob {
                paths: missing,
                platform: self.ctx.platform.clone(),
            };
            subscriptions.push(Subscription::run_with(job, load_icons));
        }
        Subscription::batch(subscriptions)
    }
}
