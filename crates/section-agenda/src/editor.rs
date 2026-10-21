use hub_core::agenda::quick_add::parse_time;
use hub_core::agenda::{NewRoutine, NewTask, Priority, Reminder, Task};
use hub_core::chrono::{NaiveDate, NaiveDateTime};
use hub_ui_kit::iced::widget::text_editor;

use crate::board::{CardKind, Month};
use crate::routine_form::{RoutineForm, RoutineFormMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTarget {
    New,
    Task(i64),
    Reminder(i64),
}

#[derive(Debug, Clone)]
pub enum EditorMessage {
    KindSelected(CardKind),
    TitleChanged(String),
    DatePicked(Option<NaiveDate>),
    PickerToggled,
    PickerShifted(i32),
    TimeChanged(String),
    PrioritySelected(Priority),
    NotesEdited(text_editor::Action),
    Routine(RoutineFormMessage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorOutput {
    Task(NewTask),
    Reminder {
        title: String,
        notes: String,
        at: NaiveDateTime,
    },
    Routine(NewRoutine),
}

pub fn mask_time(input: &str) -> String {
    let digits = |text: &str, max: usize| -> String {
        text.chars()
            .filter(char::is_ascii_digit)
            .take(max)
            .collect()
    };
    match input.split_once(':') {
        Some((hours, minutes)) => format!("{}:{}", digits(hours, 2), digits(minutes, 2)),
        None => {
            let all = digits(input, 4);
            if all.len() <= 2 {
                all
            } else {
                format!("{}:{}", &all[..2], &all[2..])
            }
        }
    }
}

pub struct Editor {
    pub target: EditorTarget,
    pub kind: CardKind,
    pub title: String,
    pub date: Option<NaiveDate>,
    pub time: String,
    pub priority: Priority,
    pub notes: text_editor::Content,
    pub picker: Option<Month>,
    pub routine: RoutineForm,
    pub error: Option<String>,
}

impl Editor {
    pub fn new(kind: CardKind, date: Option<NaiveDate>, today: NaiveDate) -> Self {
        Self {
            target: EditorTarget::New,
            kind,
            title: String::new(),
            date: date.or(Some(today)),
            time: String::new(),
            priority: Priority::Normal,
            notes: text_editor::Content::new(),
            picker: None,
            routine: RoutineForm::new(today),
            error: None,
        }
    }

    pub fn for_task(task: &Task, today: NaiveDate) -> Self {
        Self {
            target: EditorTarget::Task(task.id),
            kind: if task.routine_id.is_some() {
                CardKind::Routine
            } else {
                CardKind::Appointment
            },
            title: task.title.clone(),
            date: task.date,
            time: task
                .time
                .map(|t| t.format("%H:%M").to_string())
                .unwrap_or_default(),
            priority: task.priority,
            notes: text_editor::Content::with_text(&task.notes),
            picker: None,
            routine: RoutineForm::new(today),
            error: None,
        }
    }

    pub fn for_reminder(reminder: &Reminder, today: NaiveDate) -> Self {
        let at = reminder.effective_time();
        Self {
            target: EditorTarget::Reminder(reminder.id),
            kind: CardKind::Reminder,
            title: reminder.title.clone(),
            date: Some(at.date()),
            time: at.format("%H:%M").to_string(),
            priority: Priority::Normal,
            notes: text_editor::Content::with_text(&reminder.notes),
            picker: None,
            routine: RoutineForm::new(today),
            error: None,
        }
    }

    pub fn is_new(&self) -> bool {
        self.target == EditorTarget::New
    }

    pub fn creates_routine(&self) -> bool {
        self.is_new() && self.kind == CardKind::Routine
    }

    pub fn update(&mut self, message: EditorMessage) {
        match message {
            EditorMessage::KindSelected(kind) => {
                if self.is_new() {
                    self.kind = kind;
                }
            }
            EditorMessage::TitleChanged(title) => self.title = title,
            EditorMessage::DatePicked(date) => {
                self.date = date;
                self.picker = None;
            }
            EditorMessage::PickerToggled => {
                self.picker = match self.picker {
                    Some(_) => None,
                    None => Some(Month::of(self.date.unwrap_or(self.routine.start))),
                };
            }
            EditorMessage::PickerShifted(months) => {
                if let Some(month) = &mut self.picker {
                    *month = month.shift(months);
                }
            }
            EditorMessage::TimeChanged(time) => self.time = mask_time(&time),
            EditorMessage::PrioritySelected(priority) => self.priority = priority,
            EditorMessage::NotesEdited(action) => self.notes.perform(action),
            EditorMessage::Routine(message) => self.routine.update(message),
        }
        self.error = None;
    }

    pub fn notes_text(&self) -> String {
        self.notes.text().trim_end().to_string()
    }

    pub fn build(&self) -> Result<EditorOutput, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Dê um título.".to_string());
        }
        let time = match self.time.trim() {
            "" => None,
            text => Some(parse_time(text).ok_or("Hora inválida. Use por exemplo 08:00 ou 8h30.")?),
        };
        let notes = self.notes_text();
        match self.kind {
            CardKind::Routine if self.is_new() => {
                let mut form = self.routine.clone();
                form.title = title.to_string();
                form.time = self.time.clone();
                let mut routine = form.build()?;
                routine.notes = notes;
                Ok(EditorOutput::Routine(routine))
            }
            CardKind::Reminder => {
                let date = self.date.ok_or("Escolha a data do lembrete.")?;
                let time = time.ok_or("Informe a hora do lembrete.")?;
                Ok(EditorOutput::Reminder {
                    title: title.to_string(),
                    notes,
                    at: date.and_time(time),
                })
            }
            CardKind::Appointment | CardKind::Routine => Ok(EditorOutput::Task(NewTask {
                title: title.to_string(),
                notes,
                date: self.date,
                time,
                priority: self.priority,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use hub_core::chrono::NaiveTime;

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::parse_from_str("2026-10-21", "%Y-%m-%d").unwrap()
    }

    #[test]
    fn appointment_defaults_to_today_and_reads_time() {
        let mut editor = Editor::new(CardKind::Appointment, None, today());
        editor.update(EditorMessage::TitleChanged(" Reunião ".into()));
        editor.update(EditorMessage::TimeChanged("9:30".into()));
        editor.update(EditorMessage::PrioritySelected(Priority::High));
        let EditorOutput::Task(task) = editor.build().unwrap() else {
            panic!("esperava compromisso")
        };
        assert_eq!(task.title, "Reunião");
        assert_eq!(task.date, Some(today()));
        assert_eq!(task.time, NaiveTime::from_hms_opt(9, 30, 0));
        assert_eq!(task.priority, Priority::High);
    }

    #[test]
    fn reminder_needs_date_and_time() {
        let mut editor = Editor::new(CardKind::Reminder, None, today());
        editor.update(EditorMessage::TitleChanged("água".into()));
        assert!(editor.build().is_err());
        editor.update(EditorMessage::TimeChanged("15:00".into()));
        editor.update(EditorMessage::DatePicked(None));
        assert!(editor.build().is_err());
        editor.update(EditorMessage::DatePicked(Some(today())));
        assert!(matches!(editor.build(), Ok(EditorOutput::Reminder { .. })));
    }

    #[test]
    fn routine_uses_title_time_and_notes() {
        let mut editor = Editor::new(CardKind::Routine, None, today());
        editor.update(EditorMessage::TitleChanged("remédio".into()));
        editor.update(EditorMessage::TimeChanged("2000".into()));
        editor.update(EditorMessage::NotesEdited(text_editor::Action::Edit(
            text_editor::Edit::Paste(std::sync::Arc::new("depois do jantar".into())),
        )));
        let EditorOutput::Routine(routine) = editor.build().unwrap() else {
            panic!("esperava rotina")
        };
        assert_eq!(routine.title, "remédio");
        assert_eq!(routine.time, NaiveTime::from_hms_opt(20, 0, 0));
        assert_eq!(routine.notes, "depois do jantar");
    }

    #[test]
    fn kind_is_locked_when_editing() {
        let reminder = Reminder {
            id: 1,
            task_id: None,
            title: "x".into(),
            notes: "n".into(),
            fires_at: today().and_hms_opt(10, 0, 0).unwrap(),
            fired_at: None,
            snoozed_until: None,
        };
        let mut editor = Editor::for_reminder(&reminder, today());
        editor.update(EditorMessage::KindSelected(CardKind::Appointment));
        assert_eq!(editor.kind, CardKind::Reminder);
        assert_eq!(editor.notes_text(), "n");
        assert_eq!(editor.time, "10:00");
    }

    #[test]
    fn time_field_accepts_only_digits_and_colon() {
        assert_eq!(mask_time("8"), "8");
        assert_eq!(mask_time("08"), "08");
        assert_eq!(mask_time("083"), "08:3");
        assert_eq!(mask_time("0830"), "08:30");
        assert_eq!(mask_time("08301"), "08:30");
        assert_eq!(mask_time("8:30"), "8:30");
        assert_eq!(mask_time("8h30"), "83:0");
        assert_eq!(mask_time("ab12:x45z"), "12:45");
        assert_eq!(mask_time("abc"), "");
        assert_eq!(mask_time("12:345"), "12:34");
    }

    #[test]
    fn invalid_time_is_reported() {
        let mut editor = Editor::new(CardKind::Appointment, None, today());
        editor.update(EditorMessage::TitleChanged("x".into()));
        editor.update(EditorMessage::TimeChanged("2561".into()));
        assert!(editor.build().is_err());
    }
}
