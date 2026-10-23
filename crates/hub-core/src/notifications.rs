use std::sync::{Arc, Mutex, MutexGuard};

use chrono::NaiveDateTime;

#[derive(Clone, Default)]
pub struct Notifier {
    muted_until: Arc<Mutex<Option<NaiveDateTime>>>,
}

impl Notifier {
    fn lock(&self) -> MutexGuard<'_, Option<NaiveDateTime>> {
        self.muted_until.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn mute_until(&self, until: NaiveDateTime) {
        *self.lock() = Some(until);
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }

    pub fn muted_until(&self, now: NaiveDateTime) -> Option<NaiveDateTime> {
        let mut state = self.lock();
        match *state {
            Some(until) if now < until => Some(until),
            Some(_) => {
                *state = None;
                None
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::TimeDelta;

    use crate::clock::{Clock, FakeClock};
    use crate::events::HubEvent;
    use crate::platform::{FakePlatform, Notification};
    use crate::{Config, Database, HubContext, HubPaths};

    fn setup() -> (HubContext, Arc<FakePlatform>, Arc<FakeClock>) {
        let platform = Arc::new(FakePlatform::default());
        let clock = Arc::new(FakeClock::at("2026-10-21 09:00"));
        let ctx = HubContext::new(
            HubPaths::from_home("teste"),
            Config::default(),
            Database::open_in_memory().unwrap(),
            platform.clone(),
            clock.clone(),
        );
        (ctx, platform, clock)
    }

    fn note() -> Notification {
        Notification {
            title: "aviso".into(),
            ..Notification::default()
        }
    }

    #[test]
    fn shows_notifications_by_default() {
        let (ctx, platform, _) = setup();
        ctx.notify(note(), None);
        assert_eq!(platform.notifications().len(), 1);
        assert_eq!(ctx.notifications_muted_until(), None);
    }

    #[test]
    fn muted_notifications_are_not_shown() {
        let (ctx, platform, clock) = setup();
        let mut events = ctx.events.subscribe();
        ctx.mute_notifications(2);
        assert_eq!(events.try_recv().unwrap(), HubEvent::NotificationsChanged);
        assert_eq!(
            ctx.notifications_muted_until(),
            Some(clock.now() + TimeDelta::hours(2))
        );
        ctx.notify(note(), None);
        assert!(platform.notifications().is_empty());
    }

    #[test]
    fn notifications_return_after_the_limit() {
        let (ctx, platform, clock) = setup();
        ctx.mute_notifications(1);
        clock.advance(TimeDelta::minutes(59));
        ctx.notify(note(), None);
        assert!(platform.notifications().is_empty());
        clock.advance(TimeDelta::minutes(1));
        ctx.notify(note(), None);
        assert_eq!(platform.notifications().len(), 1);
        assert_eq!(ctx.notifications_muted_until(), None);
    }

    #[test]
    fn unmute_restores_notifications_immediately() {
        let (ctx, platform, _) = setup();
        ctx.mute_notifications(8);
        let mut events = ctx.events.subscribe();
        ctx.unmute_notifications();
        assert_eq!(events.try_recv().unwrap(), HubEvent::NotificationsChanged);
        ctx.notify(note(), None);
        assert_eq!(platform.notifications().len(), 1);
    }
}
