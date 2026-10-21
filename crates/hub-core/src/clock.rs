use std::sync::Mutex;

use chrono::{Local, NaiveDateTime, TimeDelta, Timelike};

pub trait Clock: Send + Sync {
    fn now(&self) -> NaiveDateTime;

    fn today(&self) -> chrono::NaiveDate {
        self.now().date()
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> NaiveDateTime {
        Local::now()
            .naive_local()
            .with_nanosecond(0)
            .unwrap_or_default()
    }
}

pub struct FakeClock {
    now: Mutex<NaiveDateTime>,
}

impl FakeClock {
    pub fn new(now: NaiveDateTime) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    pub fn at(text: &str) -> Self {
        Self::new(
            NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M")
                .expect("data inválida para FakeClock"),
        )
    }

    pub fn set(&self, now: NaiveDateTime) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) = now;
    }

    pub fn advance(&self, delta: TimeDelta) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        *now += delta;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> NaiveDateTime {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances() {
        let clock = FakeClock::at("2026-10-21 10:00");
        clock.advance(TimeDelta::minutes(90));
        assert_eq!(clock.now().to_string(), "2026-10-21 11:30:00");
        assert_eq!(clock.today().to_string(), "2026-10-21");
    }

    #[test]
    fn system_clock_has_no_subsecond_part() {
        assert_eq!(SystemClock.now().nanosecond(), 0);
    }
}
