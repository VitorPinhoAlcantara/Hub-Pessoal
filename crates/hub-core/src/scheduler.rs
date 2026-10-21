use chrono::{NaiveDateTime, NaiveTime, TimeDelta};

use crate::agenda::{self, Reminder};
use crate::context::HubContext;
use crate::events::HubEvent;
use crate::platform::Notification;

pub const ACTION_COMPLETE: &str = "concluir";
pub const ACTION_SNOOZE: &str = "adiar";
pub const SNOOZE_DELAY: TimeDelta = TimeDelta::minutes(10);
pub const MISSED_GRACE: TimeDelta = TimeDelta::minutes(2);

pub fn startup(ctx: &HubContext) -> agenda::Result<Vec<Reminder>> {
    let now = ctx.clock.now();
    let db = ctx.db();
    let created = agenda::generate_routine_tasks(db.conn(), now.date(), now)?;
    let missed = agenda::take_missed_reminders(db.conn(), now, MISSED_GRACE)?;
    drop(db);
    if created > 0 || !missed.is_empty() {
        ctx.events.publish(HubEvent::AgendaChanged);
    }
    Ok(missed)
}

pub fn tick(ctx: &HubContext) -> agenda::Result<Vec<Reminder>> {
    let now = ctx.clock.now();
    let db = ctx.db();
    let created = agenda::generate_routine_tasks(db.conn(), now.date(), now)?;
    let due = agenda::due_reminders(db.conn(), now)?;
    for reminder in &due {
        agenda::mark_reminder_fired(db.conn(), reminder.id, now)?;
    }
    drop(db);
    for reminder in &due {
        notify(ctx, reminder);
    }
    if created > 0 || !due.is_empty() {
        ctx.events.publish(HubEvent::AgendaChanged);
    }
    Ok(due)
}

pub fn next_wake(ctx: &HubContext) -> agenda::Result<NaiveDateTime> {
    let now = ctx.clock.now();
    let midnight = (now.date() + TimeDelta::days(1)).and_time(NaiveTime::MIN);
    let next = agenda::next_reminder_time(ctx.db().conn())?;
    Ok(match next {
        Some(at) if at < midnight => at.max(now),
        _ => midnight,
    })
}

fn notify(ctx: &HubContext, reminder: &Reminder) {
    let mut actions = Vec::new();
    if reminder.task_id.is_some() {
        actions.push((ACTION_COMPLETE.to_string(), "Concluir".to_string()));
    }
    actions.push((ACTION_SNOOZE.to_string(), "Adiar 10 min".to_string()));
    let notification = Notification {
        title: reminder.title.clone(),
        body: format!("Lembrete das {}", reminder.effective_time().format("%H:%M")),
        actions,
    };
    let handler_ctx = ctx.clone();
    let handler_reminder = reminder.clone();
    let result = ctx.platform.notify(
        notification,
        Some(Box::new(move |action| {
            if let Err(error) = handle_action(&handler_ctx, &handler_reminder, action.as_deref()) {
                tracing::warn!(%error, "falha ao tratar a ação da notificação");
            }
        })),
    );
    if let Err(error) = result {
        tracing::warn!(%error, lembrete = reminder.id, "falha ao mostrar a notificação");
    }
}

pub fn handle_action(
    ctx: &HubContext,
    reminder: &Reminder,
    action: Option<&str>,
) -> agenda::Result<()> {
    let now = ctx.clock.now();
    match action {
        Some(ACTION_COMPLETE) => {
            if let Some(task_id) = reminder.task_id {
                agenda::complete_task(ctx.db().conn(), task_id, now)?;
            }
        }
        Some(ACTION_SNOOZE) => {
            agenda::snooze_reminder(ctx.db().conn(), reminder.id, now + SNOOZE_DELAY)?;
        }
        Some(_) => {
            ctx.events.publish(HubEvent::ShowWindow);
            return Ok(());
        }
        None => return Ok(()),
    }
    ctx.events.publish(HubEvent::AgendaChanged);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::agenda::{NewTask, create_reminder, create_task};
    use crate::clock::{Clock, FakeClock};
    use crate::platform::FakePlatform;
    use crate::{Config, Database, HubPaths};

    fn setup(now: &str) -> (HubContext, Arc<FakePlatform>, Arc<FakeClock>) {
        let platform = Arc::new(FakePlatform::default());
        let clock = Arc::new(FakeClock::at(now));
        let ctx = HubContext::new(
            HubPaths::from_home("teste"),
            Config::default(),
            Database::open_in_memory().unwrap(),
            platform.clone(),
            clock.clone(),
        );
        (ctx, platform, clock)
    }

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn add_task(ctx: &HubContext, title: &str, due: &str) -> i64 {
        let due = at(due);
        create_task(
            ctx.db().conn(),
            NewTask {
                title: title.into(),
                date: Some(due.date()),
                time: Some(due.time()),
                ..NewTask::default()
            },
            ctx.clock.now(),
        )
        .unwrap()
        .id
    }

    #[test]
    fn next_wake_is_next_reminder_or_midnight() {
        let (ctx, _, _) = setup("2026-10-21 09:00");
        assert_eq!(next_wake(&ctx).unwrap(), at("2026-10-22 00:00"));
        add_task(&ctx, "a", "2026-10-21 10:00");
        assert_eq!(next_wake(&ctx).unwrap(), at("2026-10-21 10:00"));
        add_task(&ctx, "b", "2026-10-25 10:00");
        assert_eq!(next_wake(&ctx).unwrap(), at("2026-10-21 10:00"));
    }

    #[test]
    fn tick_fires_due_reminders_once() {
        let (ctx, platform, clock) = setup("2026-10-21 09:00");
        add_task(&ctx, "pagar conta", "2026-10-21 10:00");
        assert!(tick(&ctx).unwrap().is_empty());
        clock.advance(TimeDelta::hours(1));
        let fired = tick(&ctx).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(tick(&ctx).unwrap().is_empty());
        let notifications = platform.notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].title, "pagar conta");
        assert_eq!(notifications[0].actions.len(), 2);
    }

    #[test]
    fn complete_action_finishes_task() {
        let (ctx, platform, clock) = setup("2026-10-21 09:00");
        let id = add_task(&ctx, "x", "2026-10-21 09:30");
        clock.advance(TimeDelta::minutes(30));
        tick(&ctx).unwrap();
        let mut events = ctx.events.subscribe();
        assert!(platform.answer_notification(0, Some(ACTION_COMPLETE)));
        assert!(
            agenda::get_task(ctx.db().conn(), id)
                .unwrap()
                .is_completed()
        );
        assert_eq!(events.try_recv().unwrap(), HubEvent::AgendaChanged);
    }

    #[test]
    fn snooze_action_fires_again_after_ten_minutes() {
        let (ctx, platform, clock) = setup("2026-10-21 09:00");
        create_reminder(
            ctx.db().conn(),
            None,
            Some("beber água"),
            at("2026-10-21 09:01"),
        )
        .unwrap();
        clock.advance(TimeDelta::minutes(1));
        tick(&ctx).unwrap();
        assert_eq!(platform.notifications()[0].actions.len(), 1);
        assert!(platform.answer_notification(0, Some(ACTION_SNOOZE)));
        assert_eq!(next_wake(&ctx).unwrap(), at("2026-10-21 09:11"));
        clock.advance(TimeDelta::minutes(10));
        assert_eq!(tick(&ctx).unwrap().len(), 1);
        assert_eq!(platform.notifications().len(), 2);
    }

    #[test]
    fn startup_returns_missed_without_notifying() {
        let (ctx, platform, _) = setup("2026-10-21 09:00");
        create_reminder(ctx.db().conn(), None, Some("velho"), at("2026-10-20 18:00")).unwrap();
        let missed = startup(&ctx).unwrap();
        assert_eq!(missed.len(), 1);
        assert!(platform.notifications().is_empty());
        assert!(tick(&ctx).unwrap().is_empty());
    }

    #[test]
    fn midnight_tick_generates_routine_tasks() {
        let (ctx, _, clock) = setup("2026-10-21 23:59");
        agenda::create_routine(
            ctx.db().conn(),
            agenda::NewRoutine {
                title: "remédio".into(),
                notes: String::new(),
                recurrence: agenda::Recurrence::Daily,
                time: NaiveTime::from_hms_opt(8, 0, 0),
            },
        )
        .unwrap();
        clock.set(next_wake(&ctx).unwrap());
        tick(&ctx).unwrap();
        let view = agenda::today_view(ctx.db().conn(), clock.today()).unwrap();
        assert_eq!(view.today.len(), 1);
        assert_eq!(next_wake(&ctx).unwrap(), at("2026-10-22 08:00"));
    }
}
