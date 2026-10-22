use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::whatsapp::WireMessage;
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

fn section() -> WhatsApp {
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    WhatsApp::new(ctx)
}

fn send(section: &mut WhatsApp, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = section.update(message);
    }
}

fn incoming(id: &str, jid: &str, text: &str) -> WireMessage {
    WireMessage {
        id: id.into(),
        jid: jid.into(),
        de: jid.into(),
        nome_de: "Ana".into(),
        texto: text.into(),
        minha: false,
        em: 1_792_627_000,
        tipo: "texto".into(),
        historico: false,
    }
}

#[test]
fn starts_disconnected_and_shows_qr() {
    let mut section = section();
    assert!(matches!(section.connection(), Connection::Starting));
    send(
        &mut section,
        [Message::Bridge(BridgeEvent::Qr("2@abc".into()))],
    );
    assert!(matches!(section.connection(), Connection::Qr(_)));
    let _ = section.view();
}

#[test]
fn incoming_messages_create_conversations() {
    let mut section = section();
    send(
        &mut section,
        [
            Message::Bridge(BridgeEvent::Connected("eu@s.whatsapp.net".into())),
            Message::Bridge(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Message::Bridge(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Message::Selected("ana@s.whatsapp.net".into()),
        ],
    );
    assert_eq!(section.conversations().len(), 1);
    assert_eq!(section.selected().unwrap().title(), "Ana");
    assert_eq!(section.messages().len(), 1);
    let _ = section.view();
}

#[test]
fn conversation_list_is_saved_and_searchable() {
    let mut section = section();
    send(
        &mut section,
        [
            Message::ConversationsLoaded(Ok(vec![
                WireConversation {
                    jid: "a@s.whatsapp.net".into(),
                    nome: "Ana".into(),
                    grupo: false,
                },
                WireConversation {
                    jid: "f@g.us".into(),
                    nome: "Família".into(),
                    grupo: true,
                },
            ])),
            Message::QueryChanged("fam".into()),
        ],
    );
    let found = section.conversations();
    assert_eq!(found.len(), 1);
    assert!(found[0].group);
}

#[test]
fn alias_is_saved_and_cancel_keeps_name() {
    let mut section = section();
    send(
        &mut section,
        [
            Message::Bridge(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Message::Selected("ana@s.whatsapp.net".into()),
            Message::AliasEdit,
            Message::AliasChanged("Mãe".into()),
            Message::AliasCancelled,
        ],
    );
    assert_eq!(section.selected().unwrap().title(), "Ana");
    send(
        &mut section,
        [
            Message::AliasEdit,
            Message::AliasChanged("Mãe".into()),
            Message::AliasSaved,
        ],
    );
    assert_eq!(section.selected().unwrap().title(), "Mãe");
    assert!(section.alias().is_none());
    let _ = section.view();
}

#[test]
fn sending_requires_connection() {
    let mut section = section();
    send(
        &mut section,
        [
            Message::Bridge(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Message::Selected("ana@s.whatsapp.net".into()),
            Message::DraftChanged("tchau".into()),
            Message::Send,
        ],
    );
    assert!(!section.sending());
    send(
        &mut section,
        [Message::Sent(Ok(WireMessage {
            minha: true,
            id: "S1".into(),
            texto: "tchau".into(),
            em: 1_792_627_100,
            ..incoming("S1", "ana@s.whatsapp.net", "tchau")
        }))],
    );
    assert_eq!(section.draft(), "");
    assert!(section.messages().last().unwrap().mine);
}

#[test]
fn exit_and_retry() {
    let mut section = section();
    send(
        &mut section,
        [Message::Bridge(BridgeEvent::Exited("sem sidecar".into()))],
    );
    assert!(matches!(section.connection(), Connection::Offline(reason) if reason == "sem sidecar"));
    let _ = section.view();
    send(&mut section, [Message::Retry]);
    assert!(matches!(section.connection(), Connection::Starting));
    assert_eq!(section.generation, 1);
}

#[test]
fn logged_out_goes_back_to_pairing() {
    let mut section = section();
    send(
        &mut section,
        [
            Message::Bridge(BridgeEvent::Connected("eu".into())),
            Message::Bridge(BridgeEvent::Disconnected(SESSION_ENDED.into())),
        ],
    );
    assert!(matches!(section.connection(), Connection::WaitingQr));
}
