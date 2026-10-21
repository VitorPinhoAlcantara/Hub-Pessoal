use std::fmt;
use std::str::FromStr;

use chrono::{Datelike, NaiveDate, Weekday};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recurrence {
    Daily,
    Weekdays(Vec<Weekday>),
    MonthDay(u32),
    EveryNDays { days: u32, start: NaiveDate },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("recorrência inválida: {0}")]
pub struct InvalidRecurrence(pub String);

impl Recurrence {
    pub fn occurs_on(&self, date: NaiveDate) -> bool {
        match self {
            Recurrence::Daily => true,
            Recurrence::Weekdays(days) => days.contains(&date.weekday()),
            Recurrence::MonthDay(day) => {
                let last = last_day_of_month(date);
                date.day() == (*day).min(last)
            }
            Recurrence::EveryNDays { days, start } => {
                let diff = (date - *start).num_days();
                *days > 0 && diff >= 0 && diff % i64::from(*days) == 0
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Recurrence::Daily => "Todos os dias".to_string(),
            Recurrence::Weekdays(days) => days
                .iter()
                .map(|d| weekday_short(*d))
                .collect::<Vec<_>>()
                .join(", "),
            Recurrence::MonthDay(day) => format!("Todo dia {day}"),
            Recurrence::EveryNDays { days, .. } => format!("A cada {days} dias"),
        }
    }
}

fn last_day_of_month(date: NaiveDate) -> u32 {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(28)
}

pub fn weekday_short(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "seg",
        Weekday::Tue => "ter",
        Weekday::Wed => "qua",
        Weekday::Thu => "qui",
        Weekday::Fri => "sex",
        Weekday::Sat => "sáb",
        Weekday::Sun => "dom",
    }
}

impl fmt::Display for Recurrence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Recurrence::Daily => write!(f, "diaria"),
            Recurrence::Weekdays(days) => {
                let list: Vec<String> = days
                    .iter()
                    .map(|d| d.number_from_monday().to_string())
                    .collect();
                write!(f, "semana:{}", list.join(","))
            }
            Recurrence::MonthDay(day) => write!(f, "mes:{day}"),
            Recurrence::EveryNDays { days, start } => {
                write!(f, "cada:{days}:{}", start.format("%Y-%m-%d"))
            }
        }
    }
}

impl FromStr for Recurrence {
    type Err = InvalidRecurrence;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidRecurrence(text.to_string());
        let mut parts = text.split(':');
        match parts.next() {
            Some("diaria") => Ok(Recurrence::Daily),
            Some("semana") => {
                let mut days = parts
                    .next()
                    .ok_or_else(invalid)?
                    .split(',')
                    .map(|n| {
                        n.trim()
                            .parse::<u8>()
                            .ok()
                            .filter(|n| (1..=7).contains(n))
                            .and_then(|n| Weekday::try_from(n - 1).ok())
                            .ok_or_else(invalid)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                days.sort_by_key(|d| d.number_from_monday());
                days.dedup();
                if days.is_empty() {
                    return Err(invalid());
                }
                Ok(Recurrence::Weekdays(days))
            }
            Some("mes") => {
                let day: u32 = parts
                    .next()
                    .and_then(|d| d.parse().ok())
                    .ok_or_else(invalid)?;
                if !(1..=31).contains(&day) {
                    return Err(invalid());
                }
                Ok(Recurrence::MonthDay(day))
            }
            Some("cada") => {
                let days: u32 = parts
                    .next()
                    .and_then(|d| d.parse().ok())
                    .filter(|d| *d > 0)
                    .ok_or_else(invalid)?;
                let start = parts
                    .next()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .ok_or_else(invalid)?;
                Ok(Recurrence::EveryNDays { days, start })
            }
            _ => Err(invalid()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn round_trips_through_text() {
        for text in ["diaria", "semana:1,3,5", "mes:15", "cada:3:2026-10-21"] {
            let recurrence: Recurrence = text.parse().unwrap();
            assert_eq!(recurrence.to_string(), text);
        }
    }

    #[test]
    fn rejects_invalid_text() {
        for text in [
            "",
            "semana:",
            "semana:8",
            "mes:0",
            "mes:32",
            "cada:0:2026-01-01",
            "x",
        ] {
            assert!(text.parse::<Recurrence>().is_err(), "{text}");
        }
    }

    #[test]
    fn weekdays_match_only_listed_days() {
        let r: Recurrence = "semana:1,3".parse().unwrap();
        assert!(r.occurs_on(date("2026-10-19")));
        assert!(!r.occurs_on(date("2026-10-20")));
        assert!(r.occurs_on(date("2026-10-21")));
    }

    #[test]
    fn month_day_falls_back_to_last_day() {
        let r = Recurrence::MonthDay(31);
        assert!(r.occurs_on(date("2026-02-28")));
        assert!(r.occurs_on(date("2026-10-31")));
        assert!(!r.occurs_on(date("2026-10-30")));
    }

    #[test]
    fn every_n_days_counts_from_start() {
        let r: Recurrence = "cada:3:2026-10-21".parse().unwrap();
        assert!(!r.occurs_on(date("2026-10-18")));
        assert!(r.occurs_on(date("2026-10-21")));
        assert!(!r.occurs_on(date("2026-10-22")));
        assert!(r.occurs_on(date("2026-10-24")));
    }
}
