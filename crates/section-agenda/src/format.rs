use hub_core::agenda::recurrence::weekday_short;
use hub_core::agenda::{Routine, Task};
use hub_core::chrono::{Datelike, NaiveDate, NaiveDateTime};

pub fn short_date(date: NaiveDate) -> String {
    format!("{} {}", weekday_short(date.weekday()), date.format("%d/%m"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn short_dates_include_weekday() {
        assert_eq!(short_date(d("2026-10-21")), "qua 21/10");
        assert_eq!(short_date(d("2026-10-25")), "dom 25/10");
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
