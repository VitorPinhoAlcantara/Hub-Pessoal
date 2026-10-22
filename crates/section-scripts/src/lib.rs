pub mod form;
mod view;

use std::collections::HashMap;

use hub_core::HubContext;
use hub_core::scripts::runner::{self, CancelHandle, OutputStream, RunEvent, Status};
use hub_core::scripts::{self, Execution, Origin, Script};
use hub_ui_kit::Section;
use hub_ui_kit::iced::keyboard::{self, key};
use hub_ui_kit::iced::{Element, Event, Subscription, Task, event};

pub use form::{FormMessage, ScriptForm};

pub const HISTORY_LIMIT: usize = 30;
pub const CONSOLE_LIMIT: usize = 2000;

#[derive(Debug, Clone)]
pub struct ConsoleLine {
    pub stream: OutputStream,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Run {
    pub execution_id: Option<i64>,
    pub lines: Vec<ConsoleLine>,
    pub status: Option<Status>,
    pub origin: Origin,
    cancel: Option<CancelHandle>,
}

impl Run {
    fn new(origin: Origin, cancel: Option<CancelHandle>) -> Self {
        Self {
            execution_id: None,
            lines: Vec::new(),
            status: None,
            origin,
            cancel,
        }
    }

    pub fn is_running(&self) -> bool {
        self.status.is_none()
    }

    pub fn can_cancel(&self) -> bool {
        self.is_running() && self.cancel.is_some()
    }

    fn push(&mut self, stream: OutputStream, text: String) {
        self.lines.push(ConsoleLine { stream, text });
        if self.lines.len() > CONSOLE_LIMIT {
            let excess = self.lines.len() - CONSOLE_LIMIT;
            self.lines.drain(..excess);
        }
    }
}

pub enum Modal {
    Form(ScriptForm),
    ConfirmRun(String),
    ConfirmDelete(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Selected(String),
    RunRequested(String),
    RunConfirmed,
    Run(String, RunEvent),
    External(String, Origin, RunEvent),
    Cancel(String),
    ActiveToggled(String, bool),
    OpenFile(String),
    OpenFolder,
    NewScript,
    EditScript(String),
    Form(FormMessage),
    FormSaved,
    AskDelete(String),
    DeleteConfirmed,
    ModalClosed,
    HistorySelected(i64),
    ErrorDismissed,
}

pub struct Scripts {
    ctx: HubContext,
    scripts: Vec<Script>,
    last: HashMap<String, Execution>,
    selected: Option<String>,
    history: Vec<Execution>,
    history_selected: Option<i64>,
    runs: HashMap<String, Run>,
    modal: Option<Modal>,
    form_error: Option<String>,
    error: Option<String>,
}

impl Scripts {
    pub fn new(ctx: HubContext) -> Self {
        let mut section = Self {
            ctx,
            scripts: Vec::new(),
            last: HashMap::new(),
            selected: None,
            history: Vec::new(),
            history_selected: None,
            runs: HashMap::new(),
            modal: None,
            form_error: None,
            error: None,
        };
        section.reload();
        section
    }

    pub fn scripts(&self) -> &[Script] {
        &self.scripts
    }

    pub fn script(&self, id: &str) -> Option<&Script> {
        self.scripts.iter().find(|s| s.id == id)
    }

    pub fn last(&self, id: &str) -> Option<&Execution> {
        self.last.get(id)
    }

    pub fn selected(&self) -> Option<&Script> {
        self.selected.as_deref().and_then(|id| self.script(id))
    }

    pub fn history(&self) -> &[Execution] {
        &self.history
    }

    pub fn history_selected(&self) -> Option<&Execution> {
        self.history_selected
            .and_then(|id| self.history.iter().find(|e| e.id == id))
    }

    pub fn run(&self, id: &str) -> Option<&Run> {
        self.runs.get(id)
    }

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn form_error(&self) -> Option<&str> {
        self.form_error.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn now(&self) -> hub_core::chrono::NaiveDateTime {
        self.ctx.clock.now()
    }

    fn reload(&mut self) {
        self.scripts = self.ctx.scripts();
        match scripts::last_executions(self.ctx.db().conn()) {
            Ok(last) => self.last = last,
            Err(error) => self.error = Some(error.to_string()),
        }
        if self
            .selected
            .as_deref()
            .is_some_and(|id| self.script(id).is_none())
        {
            self.selected = None;
        }
        if self.selected.is_none() {
            self.selected = self.scripts.first().map(|s| s.id.clone());
        }
        self.load_history();
    }

    fn load_history(&mut self) {
        self.history = match &self.selected {
            Some(id) => {
                scripts::executions(self.ctx.db().conn(), id, HISTORY_LIMIT).unwrap_or_default()
            }
            None => Vec::new(),
        };
        if self
            .history_selected
            .is_some_and(|id| !self.history.iter().any(|e| e.id == id))
        {
            self.history_selected = None;
        }
    }

    fn start(&mut self, id: String) -> Task<Message> {
        if self.runs.get(&id).is_some_and(Run::is_running) {
            return Task::none();
        }
        let Some(script) = self.script(&id).cloned() else {
            return Task::none();
        };
        if !script.can_run() {
            self.error = script.problem.clone();
            return Task::none();
        }
        let (events, cancel) = runner::run(self.ctx.clone(), script, Origin::Manual);
        self.runs
            .insert(id.clone(), Run::new(Origin::Manual, Some(cancel)));
        self.selected = Some(id.clone());
        self.history_selected = None;
        self.load_history();
        Task::run(events, move |event| Message::Run(id.clone(), event))
    }

    fn apply_event(&mut self, id: &str, origin: Origin, event: RunEvent) {
        let run = self
            .runs
            .entry(id.to_string())
            .or_insert_with(|| Run::new(origin, None));
        match event {
            RunEvent::Started { execution_id } => {
                if run.status.is_some() {
                    *run = Run::new(origin, None);
                }
                run.execution_id = Some(execution_id);
            }
            RunEvent::Line { stream, text } => run.push(stream, text),
            RunEvent::Finished { status, .. } => {
                run.status = Some(status);
                run.cancel = None;
                self.reload();
            }
        }
    }

    fn save_form(&mut self) {
        let Some(Modal::Form(form)) = &self.modal else {
            return;
        };
        if let Err(error) = form.preview(self.now()) {
            self.form_error = Some(error);
            return;
        }
        let result = match &form.editing {
            None => scripts::create(&self.ctx.paths.scripts_dir, form.to_new()).map(Some),
            Some(id) => match self.script(id) {
                Some(script) => scripts::save_meta(script, &form.apply(&script.meta)).map(|_| None),
                None => Err(scripts::ScriptError::NotFound(id.clone())),
            },
        };
        match result {
            Ok(created) => {
                self.modal = None;
                self.form_error = None;
                if let Some(id) = created {
                    self.selected = Some(id);
                }
                self.reload();
            }
            Err(error) => self.form_error = Some(error.to_string()),
        }
    }
}

fn listen(
    event: Event,
    _status: event::Status,
    _window: hub_ui_kit::iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key::Named::Escape),
            ..
        }) => Some(Message::ModalClosed),
        _ => None,
    }
}

impl Section for Scripts {
    type Message = Message;

    fn title(&self) -> &str {
        "Scripts"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => self.reload(),
            Message::Selected(id) => {
                self.selected = Some(id);
                self.history_selected = None;
                self.load_history();
            }
            Message::RunRequested(id) => {
                let confirm = self.script(&id).is_some_and(|s| s.meta.confirm);
                if confirm {
                    self.modal = Some(Modal::ConfirmRun(id));
                } else {
                    return self.start(id);
                }
            }
            Message::RunConfirmed => {
                if let Some(Modal::ConfirmRun(id)) = self.modal.take() {
                    return self.start(id);
                }
            }
            Message::Run(id, event) => self.apply_event(&id, Origin::Manual, event),
            Message::External(id, origin, event) => self.apply_event(&id, origin, event),
            Message::Cancel(id) => {
                if let Some(cancel) = self.runs.get(&id).and_then(|r| r.cancel.as_ref()) {
                    cancel.cancel();
                }
            }
            Message::ActiveToggled(id, active) => {
                if let Some(script) = self.script(&id) {
                    let mut meta = script.meta.clone();
                    meta.active = active;
                    if let Err(error) = scripts::save_meta(script, &meta) {
                        self.error = Some(error.to_string());
                    }
                    self.reload();
                }
            }
            Message::OpenFile(id) => {
                let target = self
                    .script(&id)
                    .map(|s| s.file.clone().unwrap_or_else(|| s.meta_path.clone()));
                if let Some(path) = target
                    && let Err(error) = self.ctx.platform.open_path(&path)
                {
                    self.error = Some(error.to_string());
                }
            }
            Message::OpenFolder => {
                if let Err(error) = self.ctx.platform.open_path(&self.ctx.paths.scripts_dir) {
                    self.error = Some(error.to_string());
                }
            }
            Message::NewScript => {
                self.form_error = None;
                self.modal = Some(Modal::Form(ScriptForm::new()));
            }
            Message::EditScript(id) => {
                if let Some(form) = self.script(&id).map(|s| ScriptForm::edit(&id, &s.meta)) {
                    self.form_error = None;
                    self.modal = Some(Modal::Form(form));
                }
            }
            Message::Form(message) => {
                if let Some(Modal::Form(form)) = &mut self.modal {
                    form.update(message);
                    self.form_error = None;
                }
            }
            Message::FormSaved => self.save_form(),
            Message::AskDelete(id) => self.modal = Some(Modal::ConfirmDelete(id)),
            Message::DeleteConfirmed => {
                if let Some(Modal::ConfirmDelete(id)) = self.modal.take()
                    && let Some(script) = self.script(&id).cloned()
                {
                    if let Err(error) = scripts::delete(&self.ctx.paths.scripts_dir, &script) {
                        self.error = Some(error.to_string());
                    }
                    self.runs.remove(&id);
                    self.reload();
                }
            }
            Message::ModalClosed => self.modal = None,
            Message::HistorySelected(id) => {
                self.history_selected = if self.history_selected == Some(id) {
                    None
                } else {
                    Some(id)
                };
            }
            Message::ErrorDismissed => self.error = None,
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        event::listen_with(listen)
    }
}

#[cfg(test)]
mod tests;
