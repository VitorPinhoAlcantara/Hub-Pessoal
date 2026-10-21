use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Weekday};

use super::Priority;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTask {
    pub title: String,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub priority: Priority,
}

const CONNECTORS: &[&str] = &[
    "as", "a", "ao", "em", "no", "na", "dia", "para", "pra", "ate", "de", "proxima", "proximo",
];

const REMINDER_PREFIXES: &[&str] = &[
    "me lembra de ",
    "me lembre de ",
    "me lembra que ",
    "me lembre que ",
    "me lembra ",
    "me lembre ",
    "lembrar de ",
    "lembrete: ",
    "lembrete:",
    "lembrete ",
];

pub fn strip_reminder_prefix(text: &str) -> Option<&str> {
    let text = text.trim_start();
    REMINDER_PREFIXES.iter().find_map(|prefix| {
        text.get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| text[prefix.len()..].trim_start())
    })
}

pub fn reminder_time(parsed: &ParsedTask) -> Option<NaiveDateTime> {
    let date = parsed.date?;
    let time = parsed.time.or_else(|| NaiveTime::from_hms_opt(9, 0, 0))?;
    Some(date.and_time(time))
}

pub fn parse(text: &str, now: NaiveDateTime) -> ParsedTask {
    let original: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = original.iter().map(|w| normalize(w)).collect();
    let mut parser = Parser {
        words: &words,
        used: vec![false; words.len()],
        now,
        date: None,
        time: None,
        priority: Priority::Normal,
    };
    parser.run();
    let title = original
        .iter()
        .zip(&parser.used)
        .filter(|(_, used)| !**used)
        .map(|(word, _)| *word)
        .collect::<Vec<_>>()
        .join(" ");
    let title = title
        .trim_matches(|c: char| c == ',' || c.is_whitespace())
        .to_string();
    let mut date = parser.date;
    if let (None, Some(time)) = (date, parser.time) {
        date = Some(if time > now.time() {
            now.date()
        } else {
            now.date() + TimeDelta::days(1)
        });
    }
    ParsedTask {
        title: if title.is_empty() {
            text.trim().to_string()
        } else {
            title
        },
        date,
        time: parser.time,
        priority: parser.priority,
    }
}

fn normalize(word: &str) -> String {
    word.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect::<String>()
        .trim_end_matches([',', '.', ';', '?'])
        .to_string()
}

struct Parser<'a> {
    words: &'a [String],
    used: Vec<bool>,
    now: NaiveDateTime,
    date: Option<NaiveDate>,
    time: Option<NaiveTime>,
    priority: Priority,
}

impl Parser<'_> {
    fn run(&mut self) {
        let mut i = 0;
        while i < self.words.len() {
            let consumed = self.try_at(i);
            if consumed > 0 {
                self.mark(i, consumed);
                i += consumed;
            } else {
                i += 1;
            }
        }
    }

    fn word(&self, i: usize) -> Option<&str> {
        self.words.get(i).map(String::as_str)
    }

    fn mark(&mut self, start: usize, len: usize) {
        for used in &mut self.used[start..start + len] {
            *used = true;
        }
        let mut j = start;
        while j > 0 {
            let prev = j - 1;
            if self.used[prev] || !CONNECTORS.contains(&self.words[prev].as_str()) {
                break;
            }
            self.used[prev] = true;
            j = prev;
        }
    }

    fn try_at(&mut self, i: usize) -> usize {
        let today = self.now.date();
        let w = self.word(i).unwrap_or_default().to_string();
        match w.as_str() {
            "!alta" | "!!" => {
                self.priority = Priority::High;
                return 1;
            }
            "!baixa" => {
                self.priority = Priority::Low;
                return 1;
            }
            _ => {}
        }
        if self.date.is_none() {
            if w == "depois" && self.word(i + 1) == Some("de") && self.word(i + 2) == Some("amanha")
            {
                self.date = Some(today + TimeDelta::days(2));
                return 3;
            }
            if w == "amanha" {
                self.date = Some(today + TimeDelta::days(1));
                return 1;
            }
            if w == "hoje" {
                self.date = Some(today);
                return 1;
            }
            if let Some(day) = weekday(&w) {
                self.date = Some(next_weekday(today, day));
                return if self.word(i + 1) == Some("feira") {
                    2
                } else {
                    1
                };
            }
            if let Some(date) = slash_date(&w, today) {
                self.date = Some(date);
                return 1;
            }
            if w == "dia"
                && let Some(date) = self
                    .word(i + 1)
                    .and_then(|n| n.parse::<u32>().ok())
                    .and_then(|n| day_of_month(today, n))
            {
                self.date = Some(date);
                return 2;
            }
            if let Some(consumed) = self.relative(i) {
                return consumed;
            }
        }
        if self.time.is_none() {
            if w == "meio-dia" || (w == "meio" && self.word(i + 1) == Some("dia")) {
                self.time = NaiveTime::from_hms_opt(12, 0, 0);
                return if w == "meio" { 2 } else { 1 };
            }
            if w == "meia-noite" || (w == "meia" && self.word(i + 1) == Some("noite")) {
                self.time = NaiveTime::from_hms_opt(0, 0, 0);
                return if w == "meia" { 2 } else { 1 };
            }
            if let Some(time) = clock_time(&w) {
                self.time = Some(time);
                return 1;
            }
            if w == "as"
                && let Some(time) = self
                    .word(i + 1)
                    .and_then(|n| n.parse::<u32>().ok())
                    .and_then(|h| NaiveTime::from_hms_opt(h, 0, 0))
            {
                self.time = Some(time);
                return 2;
            }
        }
        0
    }

    fn relative(&mut self, i: usize) -> Option<usize> {
        let (offset, number_at) = match self.word(i)? {
            "em" => (1, i + 1),
            "daqui" if self.word(i + 1) == Some("a") => (2, i + 2),
            _ => return None,
        };
        let amount: i64 = self.word(number_at)?.parse().ok()?;
        let unit = self.word(number_at + 1)?;
        let delta = match unit {
            "minuto" | "minutos" | "min" => TimeDelta::minutes(amount),
            "hora" | "horas" | "h" => TimeDelta::hours(amount),
            "dia" | "dias" => {
                self.date = Some(self.now.date() + TimeDelta::days(amount));
                return Some(offset + 2);
            }
            "semana" | "semanas" => {
                self.date = Some(self.now.date() + TimeDelta::weeks(amount));
                return Some(offset + 2);
            }
            _ => return None,
        };
        if self.time.is_some() {
            return None;
        }
        let at = self.now + delta;
        self.date = Some(at.date());
        self.time = Some(at.time());
        Some(offset + 2)
    }
}

fn weekday(word: &str) -> Option<Weekday> {
    let base = word.strip_suffix("-feira").unwrap_or(word);
    Some(match base {
        "segunda" | "seg" => Weekday::Mon,
        "terca" | "ter" => Weekday::Tue,
        "quarta" | "qua" => Weekday::Wed,
        "quinta" | "qui" => Weekday::Thu,
        "sexta" | "sex" => Weekday::Fri,
        "sabado" | "sab" => Weekday::Sat,
        "domingo" | "dom" => Weekday::Sun,
        _ => return None,
    })
}

fn next_weekday(today: NaiveDate, target: Weekday) -> NaiveDate {
    let mut date = today + TimeDelta::days(1);
    while date.weekday() != target {
        date += TimeDelta::days(1);
    }
    date
}

fn slash_date(word: &str, today: NaiveDate) -> Option<NaiveDate> {
    let parts: Vec<&str> = word.split('/').collect();
    let (day, month, year) = match parts.as_slice() {
        [d, m] => (d.parse().ok()?, m.parse().ok()?, None),
        [d, m, y] => {
            let y: i32 = y.parse().ok()?;
            (
                d.parse().ok()?,
                m.parse().ok()?,
                Some(if y < 100 { 2000 + y } else { y }),
            )
        }
        _ => return None,
    };
    match year {
        Some(year) => NaiveDate::from_ymd_opt(year, month, day),
        None => {
            let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
            if this_year < today {
                NaiveDate::from_ymd_opt(today.year() + 1, month, day)
            } else {
                Some(this_year)
            }
        }
    }
}

fn day_of_month(today: NaiveDate, day: u32) -> Option<NaiveDate> {
    let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), day);
    match this_month {
        Some(date) if date >= today => Some(date),
        _ => {
            let (year, month) = if today.month() == 12 {
                (today.year() + 1, 1)
            } else {
                (today.year(), today.month() + 1)
            };
            NaiveDate::from_ymd_opt(year, month, day)
        }
    }
}

pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let word = normalize(text.trim());
    if let Ok(hours) = word.parse::<u32>() {
        return NaiveTime::from_hms_opt(hours, 0, 0);
    }
    clock_time(&word)
}

fn clock_time(word: &str) -> Option<NaiveTime> {
    let (hours, minutes) = if let Some((h, m)) = word.split_once(':') {
        (h, m)
    } else if let Some((h, m)) = word.split_once('h') {
        (h, m.trim_end_matches('s'))
    } else {
        return None;
    };
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = if minutes.is_empty() {
        0
    } else {
        minutes.parse().ok()?
    };
    NaiveTime::from_hms_opt(hours, minutes, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> NaiveDateTime {
        NaiveDateTime::parse_from_str("2026-10-21 14:00", "%Y-%m-%d %H:%M").unwrap()
    }

    fn check(text: &str, title: &str, date: Option<&str>, time: Option<&str>) {
        let parsed = parse(text, now());
        assert_eq!(parsed.title, title, "título de {text:?}");
        assert_eq!(
            parsed
                .date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .as_deref(),
            date,
            "data de {text:?}"
        );
        assert_eq!(
            parsed
                .time
                .map(|t| t.format("%H:%M").to_string())
                .as_deref(),
            time,
            "hora de {text:?}"
        );
    }

    #[test]
    fn example_from_spec() {
        check(
            "pagar conta amanhã 10h",
            "pagar conta",
            Some("2026-10-22"),
            Some("10:00"),
        );
    }

    #[test]
    fn relative_days() {
        check(
            "ligar pro banco hoje",
            "ligar pro banco",
            Some("2026-10-21"),
            None,
        );
        check(
            "revisar depois de amanhã",
            "revisar",
            Some("2026-10-23"),
            None,
        );
        check("viagem em 3 dias", "viagem", Some("2026-10-24"), None);
        check("prova daqui a 2 semanas", "prova", Some("2026-11-04"), None);
    }

    #[test]
    fn relative_minutes_and_hours() {
        check(
            "beber água em 1 minuto",
            "beber água",
            Some("2026-10-21"),
            Some("14:01"),
        );
        check("sair em 2 horas", "sair", Some("2026-10-21"), Some("16:00"));
    }

    #[test]
    fn weekdays_point_to_next_occurrence() {
        check(
            "reunião sexta 9h30",
            "reunião",
            Some("2026-10-23"),
            Some("09:30"),
        );
        check(
            "academia na quarta-feira",
            "academia",
            Some("2026-10-28"),
            None,
        );
        check("feira no sábado", "feira", Some("2026-10-24"), None);
    }

    #[test]
    fn explicit_dates() {
        check("aniversário 25/12", "aniversário", Some("2026-12-25"), None);
        check(
            "consulta 05/01 às 8h",
            "consulta",
            Some("2027-01-05"),
            Some("08:00"),
        );
        check(
            "documento 10/11/2027",
            "documento",
            Some("2027-11-10"),
            None,
        );
        check("aluguel dia 5", "aluguel", Some("2026-11-05"), None);
        check("aluguel dia 30", "aluguel", Some("2026-10-30"), None);
    }

    #[test]
    fn time_alone_uses_today_or_tomorrow() {
        check("almoço às 12", "almoço", Some("2026-10-22"), Some("12:00"));
        check("call 15:30", "call", Some("2026-10-21"), Some("15:30"));
        check(
            "dormir meia-noite",
            "dormir",
            Some("2026-10-22"),
            Some("00:00"),
        );
        check(
            "almoçar ao meio dia",
            "almoçar",
            Some("2026-10-22"),
            Some("12:00"),
        );
    }

    #[test]
    fn unrecognized_text_becomes_task_without_date() {
        check("comprar pão", "comprar pão", None, None);
        check("ler 3 capítulos", "ler 3 capítulos", None, None);
    }

    #[test]
    fn parse_time_accepts_common_formats() {
        for (text, expected) in [
            ("8", "08:00"),
            ("08:30", "08:30"),
            ("8h", "08:00"),
            ("21h15", "21:15"),
        ] {
            assert_eq!(
                parse_time(text).unwrap().format("%H:%M").to_string(),
                expected
            );
        }
        assert!(parse_time("25h").is_none());
        assert!(parse_time("abc").is_none());
    }

    #[test]
    fn reminder_prefixes_are_detected() {
        assert_eq!(
            strip_reminder_prefix("Me lembra de beber água em 1 minuto"),
            Some("beber água em 1 minuto")
        );
        assert_eq!(
            strip_reminder_prefix("lembrete: tomar café 10h"),
            Some("tomar café 10h")
        );
        assert_eq!(strip_reminder_prefix("lembrar de ligar"), Some("ligar"));
        assert_eq!(strip_reminder_prefix("pagar conta"), None);
        assert_eq!(strip_reminder_prefix("lembretes antigos"), None);
    }

    #[test]
    fn reminder_time_defaults_to_nine_when_only_date() {
        let only_date = parse("ligar amanhã", now());
        assert_eq!(
            reminder_time(&only_date).unwrap().to_string(),
            "2026-10-22 09:00:00"
        );
        let with_time = parse("ligar em 10 minutos", now());
        assert_eq!(
            reminder_time(&with_time).unwrap().to_string(),
            "2026-10-21 14:10:00"
        );
        assert!(reminder_time(&parse("ligar", now())).is_none());
    }

    #[test]
    fn priority_markers() {
        let parsed = parse("pagar boleto !alta amanhã", now());
        assert_eq!(parsed.priority, Priority::High);
        assert_eq!(parsed.title, "pagar boleto");
        assert_eq!(parse("ver série !baixa", now()).priority, Priority::Low);
    }
}
