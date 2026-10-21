use hub_core::agenda::recurrence::weekday_short;
use hub_core::agenda::{Routine, Task};
use hub_core::chrono::{Datelike, NaiveDate, NaiveDateTime, TimeDelta};

pub fn day_label(date: NaiveDate, today: NaiveDate) -> String {
    let short = format!("{} {}", weekday_short(date.weekday()), date.format("%d/%m"));
    match (date - today).num_days() {
        0 => format!("Hoje, {short}"),
        1 => format!("Amanhã, {short}"),
        -1 => format!("Ontem, {short}"),
        _ => short,
    }
}

pub fn task_meta(task: &Task, today: NaiveDate, show_date: bool) -> String {
    let mut parts = Vec::new();
    if show_date && let Some(date) = task.date {
        parts.push(day_label(date, today));
    }
    if let Some(time) = task.time {
        parts.push(time.format("%H:%M").to_string());
    }
    parts.join(", ")
}

pub fn completed_meta(task: &Task, today: NaiveDate) -> String {
    match task.completed_at {
        Some(at) => format!("Concluída {}", moment(at, today)),
        None => String::new(),
    }
}

pub fn moment(at: NaiveDateTime, today: NaiveDate) -> String {
    let time = at.format("%H:%M");
    match (today - at.date()).num_days() {
        0 => format!("hoje às {time}"),
        1 => format!("ontem às {time}"),
        _ => format!("em {} às {time}", at.format("%d/%m")),
    }
}

pub fn routine_meta(routine: &Routine) -> String {
    match routine.time {
        Some(time) => format!(
            "{}, às {}",
            routine.recurrence.describe(),
            time.format("%H:%M")
        ),
        None => routine.recurrence.describe(),
    }
}

pub fn is_tomorrow(date: NaiveDate, today: NaiveDate) -> bool {
    date - today == TimeDelta::days(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn day_labels_are_relative() {
        let today = d("2026-10-21");
        assert_eq!(day_label(today, today), "Hoje, qua 21/10");
        assert_eq!(day_label(d("2026-10-22"), today), "Amanhã, qui 22/10");
        assert_eq!(day_label(d("2026-10-20"), today), "Ontem, ter 20/10");
        assert_eq!(day_label(d("2026-10-25"), today), "dom 25/10");
        assert!(is_tomorrow(d("2026-10-22"), today));
    }

    #[test]
    fn moments_mention_day_and_time() {
        let today = d("2026-10-21");
        let at = |t: &str| NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M").unwrap();
        assert_eq!(moment(at("2026-10-21 09:05"), today), "hoje às 09:05");
        assert_eq!(moment(at("2026-10-20 18:00"), today), "ontem às 18:00");
        assert_eq!(moment(at("2026-10-01 07:00"), today), "em 01/10 às 07:00");
    }
}
