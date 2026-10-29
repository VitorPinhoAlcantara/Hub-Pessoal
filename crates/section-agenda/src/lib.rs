pub mod board;
pub mod editor;
mod format;
mod routine_form;
mod view;

use hub_core::HubContext;
use hub_core::agenda::{self, Reminder, Routine, Task};
use hub_core::chrono::{NaiveDate, TimeDelta};
use hub_core::events::HubEvent;
use hub_ui_kit::iced::keyboard::{self, Modifiers, key};
use hub_ui_kit::iced::{Element, Event, Point, Subscription, Task as Command, event, mouse};
use hub_ui_kit::{ContextMenu, Icon, MenuItem, Section};

use board::{Card, CardId, CardKind, DayColumn, Month};
pub use editor::{Editor, EditorMessage, EditorOutput, EditorTarget};
pub use routine_form::{RecurrenceKind, RoutineForm, RoutineFormMessage};

pub const COMPLETED_LIMIT: usize = 200;
pub const DRAG_THRESHOLD: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    pub card: CardId,
    pub start: Point,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Week,
    Calendar,
    Routines,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTarget {
    Card(CardId),
    Routine(i64),
}

pub enum Modal {
    Editor(Box<Editor>),
    ConfirmDelete { target: DeleteTarget, title: String },
}

#[derive(Debug, Clone)]
pub enum Message {
    ScreenSelected(Screen),
    OpenCreate(Option<NaiveDate>),
    OpenCreateRoutine,
    OpenCard(CardId),
    Editor(EditorMessage),
    EditorSaved,
    AskDelete(DeleteTarget, String),
    DeleteConfirmed,
    ModalClosed,
    TaskToggled(i64, bool),
    TaskMissed(i64, bool),
    OpenRoutine(i64),
    FilterToggled(CardKind),
    ModifiersChanged(Modifiers),
    ShiftDays(i64),
    GoToday,
    DayOpened(NaiveDate),
    MonthShifted(i32),
    RoutineToggled(i64, bool),
    MissedDismissed,
    ErrorDismissed,
    Refresh,
    CardPressed(CardId),
    DragMoved(Point),
    DragReleased,
    DayHovered(NaiveDate),
    DayLeft(NaiveDate),
    CardMenu(CardId),
    DayMenu(Option<NaiveDate>),
    MenuClosed,
    MoveCard(CardId, NaiveDate),
    OpenCreateKind(CardKind, Option<NaiveDate>),
}

pub struct Agenda {
    ctx: HubContext,
    screen: Screen,
    today: NaiveDate,
    focus: NaiveDate,
    month: Month,
    days: Vec<DayColumn>,
    pending: Vec<Card>,
    calendar: Vec<DayColumn>,
    routines: Vec<Routine>,
    completed: Vec<Task>,
    missed: Vec<Reminder>,
    filter: [bool; 3],
    modifiers: Modifiers,
    modal: Option<Modal>,
    error: Option<String>,
    drag: Option<Drag>,
    hovered_day: Option<NaiveDate>,
    menu: Option<ContextMenu<Message>>,
}

fn kind_index(kind: CardKind) -> usize {
    match kind {
        CardKind::Appointment => 0,
        CardKind::Routine => 1,
        CardKind::Reminder => 2,
    }
}

impl Agenda {
    pub fn new(ctx: HubContext, missed: Vec<Reminder>) -> Self {
        let today = ctx.clock.today();
        let mut agenda = Self {
            ctx,
            screen: Screen::Week,
            today,
            focus: today,
            month: Month::of(today),
            days: Vec::new(),
            pending: Vec::new(),
            calendar: Vec::new(),
            routines: Vec::new(),
            completed: Vec::new(),
            missed,
            filter: [false; 3],
            modifiers: Modifiers::default(),
            modal: None,
            error: None,
            drag: None,
            hovered_day: None,
            menu: None,
        };
        agenda.reload();
        agenda
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn today(&self) -> NaiveDate {
        self.today
    }

    pub fn focus(&self) -> NaiveDate {
        self.focus
    }

    pub fn month(&self) -> Month {
        self.month
    }

    pub fn days(&self) -> &[DayColumn] {
        &self.days
    }

    pub fn day(&self, date: NaiveDate) -> Option<&DayColumn> {
        self.days.iter().find(|d| d.date == date)
    }

    pub fn pending(&self) -> &[Card] {
        &self.pending
    }

    pub fn calendar(&self) -> &[DayColumn] {
        &self.calendar
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

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn editor(&self) -> Option<&Editor> {
        match &self.modal {
            Some(Modal::Editor(editor)) => Some(editor),
            _ => None,
        }
    }

    pub fn is_filtered(&self, kind: CardKind) -> bool {
        self.filter[kind_index(kind)]
    }

    pub fn has_filter(&self) -> bool {
        self.filter.iter().any(|on| *on)
    }

    pub fn shows(&self, kind: CardKind) -> bool {
        !self.has_filter() || self.is_filtered(kind)
    }

    fn reload(&mut self) {
        let today = self.ctx.clock.today();
        if today != self.today {
            if self.focus == self.today {
                self.focus = today;
            }
            self.today = today;
        }
        let (from, to) = board::week_range(self.focus);
        let grid = self.month.grid();
        let result = (|| -> agenda::Result<()> {
            let db = self.ctx.db();
            let conn = db.conn();
            self.days = board::build_days(
                from,
                to,
                &agenda::tasks_between(conn, from, to, true)?,
                &agenda::loose_reminders_between(conn, from, to)?,
                &agenda::routine_projections(conn, from, to, today)?,
            );
            self.pending = agenda::pending_before(conn, today - TimeDelta::days(board::PAST_DAYS))?
                .iter()
                .map(Card::from_task)
                .collect();
            if let (Some(first), Some(last)) = (grid.first(), grid.last()) {
                self.calendar = board::build_days(
                    *first,
                    *last,
                    &agenda::tasks_between(conn, *first, *last, true)?,
                    &agenda::loose_reminders_between(conn, *first, *last)?,
                    &agenda::routine_projections(conn, *first, *last, today)?,
                );
            }
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

    fn apply(&mut self, change: impl FnOnce(&HubContext) -> agenda::Result<()>) -> bool {
        match change(&self.ctx) {
            Ok(()) => {
                self.error = None;
                self.reload();
                self.ctx.events.publish(HubEvent::AgendaChanged);
                true
            }
            Err(error) => {
                self.fail(error);
                false
            }
        }
    }

    fn open_card(&mut self, id: CardId) {
        let today = self.today;
        let loaded = {
            let db = self.ctx.db();
            match id {
                CardId::Task(id) => {
                    agenda::get_task(db.conn(), id).map(|task| Editor::for_task(&task, today))
                }
                CardId::Reminder(id) => agenda::get_reminder(db.conn(), id)
                    .map(|reminder| Editor::for_reminder(&reminder, today)),
                CardId::Projection { routine, .. } => agenda::get_routine(db.conn(), routine)
                    .map(|routine| Editor::for_routine(&routine, today)),
            }
        };
        match loaded {
            Ok(editor) => self.modal = Some(Modal::Editor(Box::new(editor))),
            Err(error) => self.fail(error),
        }
    }

    fn save_editor(&mut self) {
        let Some(Modal::Editor(editor)) = &mut self.modal else {
            return;
        };
        let output = match editor.build() {
            Ok(output) => output,
            Err(message) => {
                editor.error = Some(message);
                return;
            }
        };
        let target = editor.target;
        let now = self.ctx.clock.now();
        let result = (|| -> agenda::Result<()> {
            let db = self.ctx.db();
            let conn = db.conn();
            match (target, output) {
                (EditorTarget::New, EditorOutput::Task(task)) => {
                    agenda::create_task(conn, task, now)?;
                }
                (EditorTarget::New, EditorOutput::Reminder { title, notes, at }) => {
                    let reminder = agenda::create_reminder(conn, None, Some(&title), at)?;
                    agenda::update_reminder(conn, reminder.id, &title, &notes, at, now)?;
                }
                (EditorTarget::New, EditorOutput::Routine(routine)) => {
                    agenda::create_routine(conn, routine)?;
                    agenda::generate_routine_tasks(conn, now.date(), now)?;
                }
                (EditorTarget::Task(id), EditorOutput::Task(task)) => {
                    agenda::update_task(conn, id, task)?;
                }
                (EditorTarget::Reminder(id), EditorOutput::Reminder { title, notes, at }) => {
                    agenda::update_reminder(conn, id, &title, &notes, at, now)?;
                }
                (EditorTarget::Routine(id), EditorOutput::Routine(routine)) => {
                    agenda::update_routine(conn, id, routine)?;
                }
                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.modal = None;
                self.error = None;
                self.reload();
                self.ctx.events.publish(HubEvent::AgendaChanged);
            }
            Err(error) => {
                if let Some(Modal::Editor(editor)) = &mut self.modal {
                    editor.error = Some(error.to_string());
                }
            }
        }
    }

    pub fn drag(&self) -> Option<&Drag> {
        self.drag.as_ref()
    }

    pub fn hovered_day(&self) -> Option<NaiveDate> {
        self.hovered_day
    }

    pub fn card(&self, id: CardId) -> Option<&Card> {
        self.days
            .iter()
            .chain(self.calendar.iter())
            .flat_map(|d| d.cards.iter())
            .chain(self.pending.iter())
            .find(|c| c.id == id)
    }

    fn move_card(&mut self, id: CardId, date: NaiveDate) {
        let now = self.ctx.clock.now();
        self.apply(|ctx| {
            let db = ctx.db();
            match id {
                CardId::Task(id) => agenda::move_task(db.conn(), id, Some(date)).map(|_| ()),
                CardId::Reminder(id) => agenda::move_reminder(db.conn(), id, date, now).map(|_| ()),
                CardId::Projection { .. } => Ok(()),
            }
        });
    }

    fn card_menu(&self, id: CardId) -> Option<ContextMenu<Message>> {
        let card = self.card(id)?.clone();
        let mut items = Vec::new();
        if card.projected {
            items.push(
                MenuItem::new("Editar a rotina", Message::OpenCard(id)).icon(Icon::Edit),
            );
            items.push(
                MenuItem::new("Ver rotinas", Message::ScreenSelected(Screen::Routines))
                    .icon(Icon::Repeat),
            );
            items.push(
                MenuItem::new(
                    "Excluir a rotina",
                    Message::AskDelete(DeleteTarget::Card(id), card.title.clone()),
                )
                .icon(Icon::Trash)
                .danger(),
            );
            return Some(ContextMenu::at_cursor(items, Message::MenuClosed));
        }
        items.push(MenuItem::new("Abrir", Message::OpenCard(id)).icon(Icon::Edit));
        if let CardId::Task(task) = id {
            let label = if card.done { "Reabrir" } else { "Concluir" };
            items.push(
                MenuItem::new(label, Message::TaskToggled(task, !card.done)).icon(Icon::Check),
            );
            if card.missed {
                items.push(
                    MenuItem::new("Desmarcar perdido", Message::TaskMissed(task, false))
                        .icon(Icon::Undo),
                );
            } else if card.is_overdue(self.today) {
                items.push(
                    MenuItem::new("Marcar como perdido", Message::TaskMissed(task, true))
                        .icon(Icon::Close),
                );
            }
        }
        if card.can_move()
            && let Some(date) = card.date
        {
            items.push(
                MenuItem::new(
                    "Adiar para amanhã",
                    Message::MoveCard(id, date + TimeDelta::days(1)),
                )
                .icon(Icon::Next),
            );
            if date != self.today {
                items.push(
                    MenuItem::new("Mover para hoje", Message::MoveCard(id, self.today))
                        .icon(Icon::Sun),
                );
            }
        }
        items.push(
            MenuItem::new(
                "Excluir",
                Message::AskDelete(DeleteTarget::Card(id), card.title),
            )
            .icon(Icon::Trash)
            .danger(),
        );
        Some(ContextMenu::at_cursor(items, Message::MenuClosed))
    }

    fn day_menu(&self, date: Option<NaiveDate>) -> ContextMenu<Message> {
        let suffix = date
            .map(|d| format!(" em {}", format::short_date(d)))
            .unwrap_or_default();
        ContextMenu::at_cursor(
            vec![
                MenuItem::new(
                    format!("Novo compromisso{suffix}"),
                    Message::OpenCreateKind(CardKind::Appointment, date),
                )
                .icon(Icon::Agenda),
                MenuItem::new(
                    format!("Novo lembrete{suffix}"),
                    Message::OpenCreateKind(CardKind::Reminder, date),
                )
                .icon(Icon::Bell),
                MenuItem::new(
                    "Nova rotina",
                    Message::OpenCreateKind(CardKind::Routine, None),
                )
                .icon(Icon::Repeat),
            ],
            Message::MenuClosed,
        )
    }

    fn delete(&mut self, target: DeleteTarget) {
        self.apply(|ctx| {
            let db = ctx.db();
            match target {
                DeleteTarget::Card(CardId::Task(id)) => agenda::delete_task(db.conn(), id),
                DeleteTarget::Card(CardId::Reminder(id)) => agenda::delete_reminder(db.conn(), id),
                DeleteTarget::Card(CardId::Projection { routine, .. }) => {
                    agenda::delete_routine(db.conn(), routine)
                }
                DeleteTarget::Routine(id) => agenda::delete_routine(db.conn(), id),
            }
        });
    }
}

fn drag_events(
    event: Event,
    _status: event::Status,
    _window: hub_ui_kit::iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::DragMoved(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::DragReleased)
        }
        _ => None,
    }
}

fn listen(
    event: Event,
    _status: event::Status,
    _window: hub_ui_kit::iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::ModifiersChanged(modifiers))
        }
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key::Named::Escape),
            ..
        }) => Some(Message::ModalClosed),
        _ => None,
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
                self.menu = None;
                self.screen = screen;
            }
            Message::OpenCreate(date) => {
                let editor = Editor::new(CardKind::Appointment, date, self.today);
                self.modal = Some(Modal::Editor(Box::new(editor)));
            }
            Message::OpenCreateRoutine => {
                let editor = Editor::new(CardKind::Routine, None, self.today);
                self.modal = Some(Modal::Editor(Box::new(editor)));
            }
            Message::OpenRoutine(id) => {
                let loaded = agenda::get_routine(self.ctx.db().conn(), id);
                match loaded {
                    Ok(routine) => {
                        let editor = Editor::for_routine(&routine, self.today);
                        self.modal = Some(Modal::Editor(Box::new(editor)));
                    }
                    Err(error) => self.fail(error),
                }
            }
            Message::OpenCard(id) => {
                self.menu = None;
                self.open_card(id);
            }
            Message::Editor(message) => {
                if let Some(Modal::Editor(editor)) = &mut self.modal {
                    editor.update(message);
                }
            }
            Message::EditorSaved => self.save_editor(),
            Message::AskDelete(target, title) => {
                self.menu = None;
                self.modal = Some(Modal::ConfirmDelete { target, title });
            }
            Message::DeleteConfirmed => {
                if let Some(Modal::ConfirmDelete { target, .. }) = self.modal.take() {
                    self.delete(target);
                }
            }
            Message::ModalClosed => self.modal = None,
            Message::TaskToggled(id, done) => {
                self.menu = None;
                let now = self.ctx.clock.now();
                self.apply(|ctx| {
                    if done {
                        agenda::complete_task(ctx.db().conn(), id, now)
                    } else {
                        agenda::uncomplete_task(ctx.db().conn(), id)
                    }
                });
            }
            Message::TaskMissed(id, missed) => {
                self.menu = None;
                let at = missed.then(|| self.ctx.clock.now());
                self.apply(|ctx| agenda::set_task_missed(ctx.db().conn(), id, at));
            }
            Message::FilterToggled(kind) => {
                if self.modifiers.shift() {
                    self.filter = [false; 3];
                } else {
                    let slot = &mut self.filter[kind_index(kind)];
                    *slot = !*slot;
                }
            }
            Message::ModifiersChanged(modifiers) => self.modifiers = modifiers,
            Message::ShiftDays(days) => {
                self.focus += TimeDelta::days(days);
                self.reload();
            }
            Message::GoToday => {
                self.focus = self.today;
                self.month = Month::of(self.today);
                self.reload();
            }
            Message::DayOpened(date) => {
                self.focus = date;
                self.screen = Screen::Week;
                self.reload();
            }
            Message::MonthShifted(months) => {
                self.month = self.month.shift(months);
                self.reload();
            }
            Message::RoutineToggled(id, active) => {
                let now = self.ctx.clock.now();
                self.apply(|ctx| {
                    let db = ctx.db();
                    agenda::set_routine_active(db.conn(), id, active)?;
                    if active {
                        agenda::generate_routine_tasks(db.conn(), now.date(), now)?;
                    }
                    Ok(())
                });
            }
            Message::MissedDismissed => self.missed.clear(),
            Message::ErrorDismissed => self.error = None,
            Message::Refresh => self.reload(),
            Message::CardPressed(card) => {
                self.menu = None;
                self.drag = Some(Drag {
                    card,
                    start: hub_ui_kit::cursor::position(),
                    active: false,
                });
            }
            Message::DragMoved(position) => {
                let pending = self
                    .drag
                    .filter(|d| !d.active && d.start.distance(position) > DRAG_THRESHOLD);
                if let Some(drag) = pending {
                    let movable = self.card(drag.card).is_some_and(|c| c.can_move());
                    self.drag = movable.then_some(Drag {
                        active: true,
                        ..drag
                    });
                }
            }
            Message::DragReleased => {
                if let Some(drag) = self.drag.take() {
                    if drag.active {
                        let source = self.card(drag.card).and_then(|c| c.date);
                        if let Some(target) = self.hovered_day
                            && Some(target) != source
                        {
                            self.move_card(drag.card, target);
                        }
                    } else {
                        self.open_card(drag.card);
                    }
                }
            }
            Message::DayHovered(date) => self.hovered_day = Some(date),
            Message::DayLeft(date) => {
                if self.hovered_day == Some(date) {
                    self.hovered_day = None;
                }
            }
            Message::CardMenu(id) => {
                self.drag = None;
                self.menu = self.card_menu(id);
            }
            Message::DayMenu(date) => self.menu = Some(self.day_menu(date)),
            Message::MenuClosed => self.menu = None,
            Message::MoveCard(id, date) => {
                self.menu = None;
                self.move_card(id, date);
            }
            Message::OpenCreateKind(kind, date) => {
                self.menu = None;
                let editor = Editor::new(kind, date, self.today);
                self.modal = Some(Modal::Editor(Box::new(editor)));
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![event::listen_with(listen)];
        if self.drag.is_some() {
            subscriptions.push(event::listen_with(drag_events));
        }
        Subscription::batch(subscriptions)
    }

    fn context_menu(&self) -> Option<ContextMenu<Message>> {
        self.menu.clone()
    }

    fn on_hide(&mut self) {
        self.modifiers = Modifiers::default();
    }
}

#[cfg(test)]
mod tests;
