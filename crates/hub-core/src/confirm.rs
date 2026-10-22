use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::events::{EventBus, HubEvent};

pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

type Pending = HashMap<u64, (ConfirmRequest, oneshot::Sender<bool>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    pub id: u64,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Default)]
pub struct ConfirmationBroker {
    next: Arc<AtomicU64>,
    pending: Arc<Mutex<Pending>>,
}

impl ConfirmationBroker {
    pub fn ask(
        &self,
        events: &EventBus,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> (u64, oneshot::Receiver<bool>) {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let request = ConfirmRequest {
            id,
            title: title.into(),
            detail: detail.into(),
        };
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, (request.clone(), tx));
        events.publish(HubEvent::ConfirmationRequested(request));
        (id, rx)
    }

    pub async fn confirm(
        &self,
        events: &EventBus,
        title: impl Into<String>,
        detail: impl Into<String>,
        timeout: Duration,
    ) -> bool {
        let (id, rx) = self.ask(events, title, detail);
        let approved = matches!(tokio::time::timeout(timeout, rx).await, Ok(Ok(true)));
        if self.forget(id) {
            events.publish(HubEvent::ConfirmationClosed(id));
        }
        approved
    }

    pub fn answer(&self, id: u64, approved: bool) -> bool {
        let entry = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        match entry {
            Some((_, tx)) => tx.send(approved).is_ok(),
            None => false,
        }
    }

    fn forget(&self, id: u64) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
            .is_some()
    }

    pub fn pending(&self) -> Vec<ConfirmRequest> {
        let mut list: Vec<ConfirmRequest> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|(request, _)| request.clone())
            .collect();
        list.sort_by_key(|r| r.id);
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn answered_request_returns_choice() {
        let broker = ConfirmationBroker::default();
        let events = EventBus::new();
        let mut rx = events.subscribe();
        let waiting = {
            let broker = broker.clone();
            let events = events.clone();
            tokio::spawn(async move {
                broker
                    .confirm(&events, "Executar", "backup", Duration::from_secs(5))
                    .await
            })
        };
        let HubEvent::ConfirmationRequested(request) = rx.recv().await.unwrap() else {
            panic!("esperava pedido")
        };
        assert_eq!(broker.pending().len(), 1);
        assert!(broker.answer(request.id, true));
        assert!(waiting.await.unwrap());
        assert!(broker.pending().is_empty());
        assert!(!broker.answer(request.id, true));
    }

    #[tokio::test]
    async fn timeout_is_a_refusal_and_closes_dialog() {
        let broker = ConfirmationBroker::default();
        let events = EventBus::new();
        let mut rx = events.subscribe();
        let approved = broker
            .confirm(&events, "Enviar", "mensagem", Duration::from_millis(50))
            .await;
        assert!(!approved);
        assert!(matches!(
            rx.recv().await.unwrap(),
            HubEvent::ConfirmationRequested(_)
        ));
        assert!(matches!(
            rx.recv().await.unwrap(),
            HubEvent::ConfirmationClosed(1)
        ));
        assert!(broker.pending().is_empty());
    }

    #[tokio::test]
    async fn refusal_returns_false() {
        let broker = ConfirmationBroker::default();
        let events = EventBus::new();
        let (id, rx) = broker.ask(&events, "x", "y");
        broker.answer(id, false);
        assert!(!rx.await.unwrap());
    }
}
