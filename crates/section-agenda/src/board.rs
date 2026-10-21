use hub_core::agenda::{Priority, Reminder, Task};
use hub_core::chrono::{Datelike, Months, NaiveDate, NaiveTime, TimeDelta, Weekday};

pub const PAST_DAYS: i64 = 2;
pub const FUTURE_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CardKind {
    Appointment,
    Routine,
    Reminder,
}

impl CardKind {
    pub const ALL: [CardKind; 3] = [CardKind::Appointment, CardKind::Routine, CardKind::Reminder];

    pub fn label(self) -> &'static str {
        match self {
            CardKind::Appointment => "Compromisso",
            CardKind::Routine => "Rotina",
            CardKind::Reminder => "Lembrete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardId {
    Task(i64),
    Reminder(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    pub kind: CardKind,
    pub title: String,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub done: bool,
    pub high_priority: bool,
}

impl Card {
    pub fn from_task(task: &Task) -> Self {
        Self {
            id: CardId::Task(task.id),
            kind: if task.routine_id.is_some() {
                CardKind::Routine
            } else {
                CardKind::Appointment
            },
            title: task.title.clone(),
            date: task.date,
            time: task.time,
            done: task.is_completed(),
            high_priority: task.priority == Priority::High,
        }
    }

    pub fn from_reminder(reminder: &Reminder) -> Self {
        let at = reminder.effective_time();
        Self {
            id: CardId::Reminder(reminder.id),
            kind: CardKind::Reminder,
            title: reminder.title.clone(),
            date: Some(at.date()),
            time: Some(at.time()),
            done: reminder.fired_at.is_some(),
            high_priority: false,
        }
    }

    pub fn is_overdue(&self, today: NaiveDate) -> bool {
        !self.done && self.kind != CardKind::Reminder && self.date.is_some_and(|date| date < today)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayColumn {
    pub date: NaiveDate,
    pub cards: Vec<Card>,
}

pub fn build_days(
    from: NaiveDate,
    to: NaiveDate,
    tasks: &[Task],
    reminders: &[Reminder],
) -> Vec<DayColumn> {
    let mut days: Vec<DayColumn> = from
        .iter_days()
        .take_while(|date| *date <= to)
        .map(|date| DayColumn {
            date,
            cards: Vec::new(),
        })
        .collect();
    let cards = tasks
        .iter()
        .map(Card::from_task)
        .chain(reminders.iter().map(Card::from_reminder));
    for card in cards {
        let Some(date) = card.date else { continue };
        if let Some(day) = days.iter_mut().find(|d| d.date == date) {
            day.cards.push(card);
        }
    }
    for day in &mut days {
        day.cards
            .sort_by_key(|card| (card.time.is_some(), card.time, card.done));
    }
    days
}

pub fn week_range(focus: NaiveDate) -> (NaiveDate, NaiveDate) {
    (
        focus - TimeDelta::days(PAST_DAYS),
        focus + TimeDelta::days(FUTURE_DAYS),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub past: usize,
    pub column_width: f32,
}

pub fn layout(available: f32, min_width: f32, gap: f32) -> Layout {
    let max_visible = (2 * PAST_DAYS + 1) as usize;
    let fit = (((available + gap) / (min_width + gap)).floor() as usize).max(1);
    let mut visible = fit.min(max_visible);
    if visible.is_multiple_of(2) {
        visible -= 1;
    }
    let past = (visible - 1) / 2;
    let column_width = ((available - gap * (visible - 1) as f32) / visible as f32).max(min_width);
    Layout { past, column_width }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Month {
    pub year: i32,
    pub month: u32,
}

impl Month {
    pub fn of(date: NaiveDate) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    pub fn first_day(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap_or_default()
    }

    pub fn shift(self, months: i32) -> Self {
        let first = self.first_day();
        let shifted = if months >= 0 {
            first.checked_add_months(Months::new(months as u32))
        } else {
            first.checked_sub_months(Months::new(months.unsigned_abs()))
        };
        Self::of(shifted.unwrap_or(first))
    }

    pub fn grid(self) -> Vec<NaiveDate> {
        let first = self.first_day();
        let start = first - TimeDelta::days(first.weekday().num_days_from_monday() as i64);
        let next = self.shift(1).first_day();
        let last = next - TimeDelta::days(1);
        let end = last + TimeDelta::days(6 - last.weekday().num_days_from_monday() as i64);
        start.iter_days().take_while(|d| *d <= end).collect()
    }

    pub fn label(self) -> String {
        const NAMES: [&str; 12] = [
            "Janeiro",
            "Fevereiro",
            "Março",
            "Abril",
            "Maio",
            "Junho",
            "Julho",
            "Agosto",
            "Setembro",
            "Outubro",
            "Novembro",
            "Dezembro",
        ];
        format!("{} de {}", NAMES[(self.month - 1) as usize], self.year)
    }
}

pub fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Segunda",
        Weekday::Tue => "Terça",
        Weekday::Wed => "Quarta",
        Weekday::Thu => "Quinta",
        Weekday::Fri => "Sexta",
        Weekday::Sat => "Sábado",
        Weekday::Sun => "Domingo",
    }
}

#[cfg(test)]
mod tests {
    use hub_core::chrono::NaiveDateTime;

    use super::*;

    fn d(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn task(id: i64, date: &str, time: Option<&str>, routine: bool) -> Task {
        Task {
            id,
            title: format!("t{id}"),
            notes: String::new(),
            date: Some(d(date)),
            time: time.map(|t| NaiveTime::parse_from_str(t, "%H:%M").unwrap()),
            priority: Priority::Normal,
            completed_at: None,
            created_at: NaiveDateTime::default(),
            routine_id: routine.then_some(1),
        }
    }

    #[test]
    fn days_get_their_cards_sorted_with_untimed_first() {
        let tasks = vec![
            task(1, "2026-10-21", Some("15:00"), false),
            task(2, "2026-10-21", None, true),
            task(3, "2026-10-21", Some("08:00"), false),
            task(4, "2026-10-30", None, false),
        ];
        let reminder = Reminder {
            id: 9,
            task_id: None,
            title: "água".into(),
            fires_at: NaiveDateTime::parse_from_str("2026-10-22 10:00", "%Y-%m-%d %H:%M").unwrap(),
            fired_at: None,
            snoozed_until: None,
            notes: String::new(),
        };
        let days = build_days(d("2026-10-21"), d("2026-10-22"), &tasks, &[reminder]);
        assert_eq!(days.len(), 2);
        let ids: Vec<CardId> = days[0].cards.iter().map(|c| c.id).collect();
        assert_eq!(ids, [CardId::Task(2), CardId::Task(3), CardId::Task(1)]);
        assert_eq!(days[0].cards[0].kind, CardKind::Routine);
        assert_eq!(days[1].cards[0].kind, CardKind::Reminder);
    }

    #[test]
    fn layout_centers_today_with_up_to_two_past_days() {
        let wide = layout(1600.0, 200.0, 8.0);
        assert_eq!(wide.past, 2);
        assert!(wide.column_width > 200.0);
        let medium = layout(900.0, 200.0, 8.0);
        assert_eq!(medium.past, 1);
        let narrow = layout(150.0, 200.0, 8.0);
        assert_eq!(narrow.past, 0);
        assert_eq!(narrow.column_width, 200.0);
    }

    #[test]
    fn month_grid_starts_monday_and_covers_whole_weeks() {
        let grid = Month {
            year: 2026,
            month: 10,
        }
        .grid();
        assert_eq!(grid.first().unwrap().to_string(), "2026-09-28");
        assert_eq!(grid.last().unwrap().to_string(), "2026-11-01");
        assert_eq!(grid.len() % 7, 0);
    }

    #[test]
    fn month_shift_crosses_years() {
        let december = Month {
            year: 2026,
            month: 12,
        };
        assert_eq!(
            december.shift(1),
            Month {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(
            Month {
                year: 2027,
                month: 1
            }
            .shift(-1),
            december
        );
        assert_eq!(december.label(), "Dezembro de 2026");
    }

    #[test]
    fn overdue_applies_to_open_tasks_only() {
        let card = Card::from_task(&task(1, "2026-10-20", None, false));
        assert!(card.is_overdue(d("2026-10-21")));
        assert!(!card.is_overdue(d("2026-10-20")));
    }
}
