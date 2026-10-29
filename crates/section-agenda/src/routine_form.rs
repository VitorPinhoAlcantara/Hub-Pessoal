use hub_core::agenda::quick_add::parse_time;
use hub_core::agenda::{NewRoutine, Recurrence, Routine};
use hub_core::chrono::{NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecurrenceKind {
    Daily,
    Weekdays,
    MonthDay,
    EveryNDays,
}

impl RecurrenceKind {
    pub const ALL: [RecurrenceKind; 4] = [
        RecurrenceKind::Daily,
        RecurrenceKind::Weekdays,
        RecurrenceKind::MonthDay,
        RecurrenceKind::EveryNDays,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RecurrenceKind::Daily => "Diária",
            RecurrenceKind::Weekdays => "Dias da semana",
            RecurrenceKind::MonthDay => "Dia do mês",
            RecurrenceKind::EveryNDays => "A cada N dias",
        }
    }
}

pub const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

#[derive(Debug, Clone)]
pub enum RoutineFormMessage {
    TitleChanged(String),
    KindSelected(RecurrenceKind),
    WeekdayToggled(usize),
    NumberChanged(String),
    TimeChanged(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutineForm {
    pub title: String,
    pub kind: RecurrenceKind,
    pub weekdays: [bool; 7],
    pub number: String,
    pub time: String,
    pub start: NaiveDate,
}

impl RoutineForm {
    pub fn new(start: NaiveDate) -> Self {
        Self {
            title: String::new(),
            kind: RecurrenceKind::Daily,
            weekdays: [true, true, true, true, true, false, false],
            number: String::new(),
            time: String::new(),
            start,
        }
    }

    pub fn from_routine(routine: &Routine, today: NaiveDate) -> Self {
        let mut form = Self::new(today);
        form.title = routine.title.clone();
        form.time = routine
            .time
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default();
        match &routine.recurrence {
            Recurrence::Daily => form.kind = RecurrenceKind::Daily,
            Recurrence::Weekdays(days) => {
                form.kind = RecurrenceKind::Weekdays;
                form.weekdays = WEEKDAYS.map(|day| days.contains(&day));
            }
            Recurrence::MonthDay(day) => {
                form.kind = RecurrenceKind::MonthDay;
                form.number = day.to_string();
            }
            Recurrence::EveryNDays { days, start } => {
                form.kind = RecurrenceKind::EveryNDays;
                form.number = days.to_string();
                form.start = *start;
            }
        }
        form
    }

    pub fn update(&mut self, message: RoutineFormMessage) {
        match message {
            RoutineFormMessage::TitleChanged(title) => self.title = title,
            RoutineFormMessage::KindSelected(kind) => {
                self.kind = kind;
                self.number.clear();
            }
            RoutineFormMessage::WeekdayToggled(index) => {
                if let Some(day) = self.weekdays.get_mut(index) {
                    *day = !*day;
                }
            }
            RoutineFormMessage::NumberChanged(number) => {
                self.number = number
                    .chars()
                    .filter(char::is_ascii_digit)
                    .take(3)
                    .collect();
            }
            RoutineFormMessage::TimeChanged(time) => self.time = time,
        }
    }

    pub fn build(&self) -> Result<NewRoutine, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Dê um nome para a rotina.".to_string());
        }
        let time = if self.time.trim().is_empty() {
            None
        } else {
            Some(parse_time(&self.time).ok_or("Hora inválida. Use por exemplo 08:00 ou 8h30.")?)
        };
        let number = || self.number.parse::<u32>().ok();
        let recurrence = match self.kind {
            RecurrenceKind::Daily => Recurrence::Daily,
            RecurrenceKind::Weekdays => {
                let days: Vec<Weekday> = WEEKDAYS
                    .iter()
                    .zip(self.weekdays)
                    .filter(|(_, on)| *on)
                    .map(|(day, _)| *day)
                    .collect();
                if days.is_empty() {
                    return Err("Escolha pelo menos um dia da semana.".to_string());
                }
                Recurrence::Weekdays(days)
            }
            RecurrenceKind::MonthDay => match number() {
                Some(day @ 1..=31) => Recurrence::MonthDay(day),
                _ => return Err("O dia do mês precisa estar entre 1 e 31.".to_string()),
            },
            RecurrenceKind::EveryNDays => match number() {
                Some(days) if days > 0 => Recurrence::EveryNDays {
                    days,
                    start: self.start,
                },
                _ => return Err("Informe a cada quantos dias.".to_string()),
            },
        };
        Ok(NewRoutine {
            title: title.to_string(),
            notes: String::new(),
            recurrence,
            time,
        })
    }
}
