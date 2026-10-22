use tokio::sync::broadcast;

use crate::confirm::ConfirmRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEvent {
    AgendaChanged,
    ScriptsChanged,
    ShortcutsChanged,
    ConfigChanged,
    ShowWindow,
    QuitRequested,
    ConfirmationRequested(ConfirmRequest),
    ConfirmationClosed(u64),
    PlayRequested(String),
    WhatsAppChanged,
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<HubEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(64);
        Self { sender }
    }

    pub fn publish(&self, event: HubEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.sender.subscribe()
    }
}

impl std::hash::Hash for EventBus {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "hub-event-bus".hash(state);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribers_receive_published_events() {
        let bus = EventBus::new();
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();
        bus.publish(HubEvent::AgendaChanged);
        assert_eq!(first.try_recv().unwrap(), HubEvent::AgendaChanged);
        assert_eq!(second.try_recv().unwrap(), HubEvent::AgendaChanged);
    }

    #[test]
    fn publishing_without_subscribers_does_not_fail() {
        EventBus::new().publish(HubEvent::ConfigChanged);
    }
}
