mod format;
mod routine_form;
mod view;

use hub_core::HubContext;
use hub_core::agenda::{self, NewRoutine, Reminder, Routine, Task, TodayView, quick_add};
use hub_core::chrono::NaiveDate;
use hub_core::events::HubEvent;
use hub_ui_kit::Section;
use hub_ui_kit::iced::{Element, Subscription, Task as Command};

pub use routine_form::{RecurrenceKind, RoutineForm, RoutineFormMessage};

pub const UPCOMING_DAYS: i64 = 14;
pub const COMPLETED_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Today,
    Upcoming,
    Routines,
    Completed,
}

#[derive(Debug, Clone)]
pub enum Message {
    ScreenSelected(Screen),
    QuickAddChanged(String),
    QuickAddSubmitted,
    TaskToggled(i64, bool),
    TaskDeleted(i64),
    RoutineToggled(i64, bool),
    RoutineDeleted(i64),
    RoutineFormOpened,
    RoutineFormClosed,
    RoutineForm(RoutineFormMessage),
    RoutineSubmitted,
    MissedDismissed,
    ErrorDismissed,
    Refresh,
}

pub struct Agenda {
    ctx: HubContext,
    screen: Screen,
    quick_add: String,
    today: TodayView,
    upcoming: Vec<(NaiveDate, Vec<Task>)>,
    routines: Vec<Routine>,
    completed: Vec<Task>,
    missed: Vec<Reminder>,
    routine_form: Option<RoutineForm>,
    error: Option<String>,
    current_day: NaiveDate,
}

impl Agenda {
    pub fn new(ctx: HubContext, missed: Vec<Reminder>) -> Self {
        let current_day = ctx.clock.today();
        let mut agenda = Self {
            ctx,
            screen: Screen::Today,
            quick_add: String::new(),
            today: TodayView::default(),
            upcoming: Vec::new(),
            routines: Vec::new(),
            completed: Vec::new(),
            missed,
            routine_form: None,
            error: None,
            current_day,
        };
        agenda.reload();
        agenda
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn today(&self) -> &TodayView {
        &self.today
    }

    pub fn upcoming(&self) -> &[(NaiveDate, Vec<Task>)] {
        &self.upcoming
    }

    pub fn routines(&self) -> &[Routine] {
        &self.routines
    }

    pub fn completed(&self) -> &[Task] {
        &self.completed
    }

    pub fn missed(&self) -> &[Reminder] {
        &self.missed
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn quick_add_text(&self) -> &str {
        &self.quick_add
    }

    pub fn routine_form(&self) -> Option<&RoutineForm> {
        self.routine_form.as_ref()
    }

    fn reload(&mut self) {
        self.current_day = self.ctx.clock.today();
        let result = (|| -> agenda::Result<()> {
            let db = self.ctx.db();
            let conn = db.conn();
            self.today = agenda::today_view(conn, self.current_day)?;
            self.upcoming = agenda::upcoming(conn, self.current_day, UPCOMING_DAYS)?;
            self.routines = agenda::list_routines(conn)?;
            self.completed = agenda::completed(conn, COMPLETED_LIMIT)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.fail(error);
        }
    }

    fn fail(&mut self, error: agenda::AgendaError) {
        tracing::warn!(%error, "erro na agenda");
        self.error = Some(error.to_string());
    }

    fn apply(&mut self, change: impl FnOnce(&HubContext) -> agenda::Result<()>) {
        match change(&self.ctx) {
            Ok(()) => {
                self.error = None;
                self.reload();
                self.ctx.events.publish(HubEvent::AgendaChanged);
            }
            Err(error) => self.fail(error),
        }
    }
}

impl Section for Agenda {
    type Message = Message;

    fn title(&self) -> &str {
        "Agenda"
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::ScreenSelected(screen) => {
                self.screen = screen;
                self.reload();
            }
            Message::QuickAddChanged(text) => self.quick_add = text,
            Message::QuickAddSubmitted => {
                let text = self.quick_add.trim().to_string();
                if text.is_empty() {
                    return Command::none();
                }
                let now = self.ctx.clock.now();
                let parsed = quick_add::parse(&text, now);
                self.apply(|ctx| {
                    agenda::create_task(ctx.db().conn(), parsed.into(), now).map(|_| ())
                });
                if self.error.is_none() {
                    self.quick_add.clear();
                }
            }
            Message::TaskToggled(id, done) => {
                let now = self.ctx.clock.now();
                self.apply(|ctx| {
                    if done {
                        agenda::complete_task(ctx.db().conn(), id, now)
                    } else {
                        agenda::uncomplete_task(ctx.db().conn(), id)
                    }
                });
            }
            Message::TaskDeleted(id) => self.apply(|ctx| agenda::delete_task(ctx.db().conn(), id)),
            Message::RoutineToggled(id, active) => {
                self.apply(|ctx| agenda::set_routine_active(ctx.db().conn(), id, active));
                if active {
                    let now = self.ctx.clock.now();
                    self.apply(|ctx| {
                        agenda::generate_routine_tasks(ctx.db().conn(), now.date(), now).map(|_| ())
                    });
                }
            }
            Message::RoutineDeleted(id) => {
                self.apply(|ctx| agenda::delete_routine(ctx.db().conn(), id))
            }
            Message::RoutineFormOpened => {
                self.routine_form = Some(RoutineForm::new(self.current_day));
            }
            Message::RoutineFormClosed => self.routine_form = None,
            Message::RoutineForm(message) => {
                if let Some(form) = &mut self.routine_form {
                    form.update(message);
                }
            }
            Message::RoutineSubmitted => {
                let Some(form) = &self.routine_form else {
                    return Command::none();
                };
                match form.build() {
                    Ok(routine) => self.create_routine(routine),
                    Err(message) => self.error = Some(message),
                }
            }
            Message::MissedDismissed => self.missed.clear(),
            Message::ErrorDismissed => self.error = None,
            Message::Refresh => self.reload(),
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }
}

impl Agenda {
    fn create_routine(&mut self, routine: NewRoutine) {
        let now = self.ctx.clock.now();
        self.apply(|ctx| {
            let db = ctx.db();
            agenda::create_routine(db.conn(), routine)?;
            agenda::generate_routine_tasks(db.conn(), now.date(), now)?;
            Ok(())
        });
        if self.error.is_none() {
            self.routine_form = None;
        }
    }
}

#[cfg(test)]
mod tests;
