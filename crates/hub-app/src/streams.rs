use std::time::Duration;

use hub_core::chrono::{Local, NaiveDateTime};
use hub_core::events::{EventBus, HubEvent};
use iced::futures::Stream;
use iced::futures::stream;
use tokio::sync::broadcast::error::RecvError;

const MAX_SLEEP: Duration = Duration::from_secs(15 * 60);

pub fn bus_events(bus: &EventBus) -> impl Stream<Item = HubEvent> + use<> {
    stream::unfold(bus.subscribe(), |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => return Some((event, receiver)),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
}

pub fn wake_at(wake: &NaiveDateTime) -> impl Stream<Item = ()> + use<> {
    let wake = *wake;
    stream::unfold(false, move |done| async move {
        if done {
            return None;
        }
        loop {
            let now = Local::now().naive_local();
            let Ok(remaining) = (wake - now).to_std() else {
                return Some(((), true));
            };
            if remaining.is_zero() {
                return Some(((), true));
            }
            tokio::time::sleep(remaining.min(MAX_SLEEP)).await;
        }
    })
}

pub fn after(duration: Duration) -> impl std::future::Future<Output = ()> {
    tokio::time::sleep(duration)
}
