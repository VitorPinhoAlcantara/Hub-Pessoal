use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::whatsapp::WireMessage;
use hub_core::whatsapp::bridge::BridgeEvent;
use hub_core::whatsapp::protocol::WireConversation;
use hub_core::whatsapp::service::Processor;
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

struct Tester {
    section: WhatsApp,
    processor: Processor,
}

enum Step {
    Bridge(BridgeEvent),
    Ui(Message),
}

impl From<Message> for Step {
    fn from(message: Message) -> Self {
        Step::Ui(message)
    }
}

impl From<BridgeEvent> for Step {
    fn from(event: BridgeEvent) -> Self {
        Step::Bridge(event)
    }
}

fn section() -> Tester {
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    let processor = Processor::new(ctx.clone(), ctx.whatsapp.clone());
    Tester {
        section: WhatsApp::new(ctx),
        processor,
    }
}

fn send(tester: &mut Tester, steps: impl IntoIterator<Item = Step>) {
    for step in steps {
        match step {
            Step::Bridge(event) => {
                let _ = tester.processor.handle(event);
                tester.processor.flush();
                let _ = tester.section.update(Message::Refresh);
            }
            Step::Ui(message) => {
                let _ = tester.section.update(message);
            }
        }
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
        midia: String::new(),
    }
}

#[test]
fn starts_disconnected_and_shows_qr() {
    let mut tester = section();
    assert!(matches!(
        tester.section.connection(),
        Connection::Offline(_)
    ));
    send(&mut tester, [Step::from(BridgeEvent::Qr("2@abc".into()))]);
    assert!(matches!(tester.section.connection(), Connection::Qr(_)));
    let _ = tester.section.view();
}

#[test]
fn incoming_messages_create_conversations() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Connected("eu@s.whatsapp.net".into())),
            Step::from(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Step::from(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Step::from(Message::Selected("ana@s.whatsapp.net".into())),
        ],
    );
    assert_eq!(tester.section.conversations().len(), 1);
    assert_eq!(tester.section.selected().unwrap().title(), "Ana");
    assert_eq!(tester.section.messages().len(), 1);
    let _ = tester.section.view();
}

#[test]
fn conversation_list_is_saved_and_searchable() {
    let mut tester = section();
    {
        let db = tester.section.ctx.db();
        for conversation in [
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
        ] {
            hub_core::whatsapp::upsert_conversation(db.conn(), &conversation).unwrap();
        }
    }
    send(
        &mut tester,
        [
            Step::from(Message::Refresh),
            Step::from(Message::QueryChanged("fam".into())),
        ],
    );
    let found = tester.section.conversations();
    assert_eq!(found.len(), 1);
    assert!(found[0].group);
}

#[test]
fn alias_is_saved_and_cancel_keeps_name() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Step::from(Message::Selected("ana@s.whatsapp.net".into())),
            Step::from(Message::AliasEdit),
            Step::from(Message::AliasChanged("Mãe".into())),
            Step::from(Message::AliasCancelled),
        ],
    );
    assert_eq!(tester.section.selected().unwrap().title(), "Ana");
    send(
        &mut tester,
        [
            Step::from(Message::AliasEdit),
            Step::from(Message::AliasChanged("Mãe".into())),
            Step::from(Message::AliasSaved),
        ],
    );
    assert_eq!(tester.section.selected().unwrap().title(), "Mãe");
    assert!(tester.section.alias().is_none());
    let _ = tester.section.view();
}

#[test]
fn sending_requires_connection() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Step::from(Message::Selected("ana@s.whatsapp.net".into())),
            Step::from(Message::DraftChanged("tchau".into())),
            Step::from(Message::Send),
        ],
    );
    assert!(!tester.section.sending());
    send(
        &mut tester,
        [Step::from(Message::Sent(Ok(WireMessage {
            minha: true,
            id: "S1".into(),
            texto: "tchau".into(),
            em: 1_792_627_100,
            ..incoming("S1", "ana@s.whatsapp.net", "tchau")
        })))],
    );
    assert_eq!(tester.section.draft(), "");
    assert!(tester.section.messages().last().unwrap().mine);
}

#[test]
fn exit_and_switch_off() {
    let mut tester = section();
    send(
        &mut tester,
        [Step::from(BridgeEvent::Exited("sem sidecar".into()))],
    );
    assert!(
        matches!(tester.section.connection(), Connection::Offline(reason) if reason == "sem sidecar")
    );
    let _ = tester.section.view();
    send(&mut tester, [Step::from(Message::SwitchOff)]);
    assert!(
        matches!(tester.section.connection(), Connection::Offline(reason) if reason == SWITCHED_OFF)
    );
}

#[test]
fn groups_notify_only_when_enabled() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(incoming("1", "123@g.us", "grupo"))),
            Step::from(Message::ConversationMenu("123@g.us".into())),
        ],
    );
    assert_eq!(tester.section.context_menu().unwrap().items.len(), 3);
    assert!(!tester.section.conversations()[0].notify);
    send(
        &mut tester,
        [Step::from(Message::ToggleNotify("123@g.us".into()))],
    );
    assert!(tester.section.conversations()[0].notify);
}

#[test]
fn logged_out_goes_back_to_pairing() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Connected("eu".into())),
            Step::from(BridgeEvent::Disconnected(SESSION_ENDED.into())),
        ],
    );
    assert!(matches!(tester.section.connection(), Connection::WaitingQr));
}

#[test]
fn right_click_offers_alias_and_missing_sidecar_has_guide() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Connected("eu@s.whatsapp.net".into())),
            Step::from(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Step::from(Message::ConversationMenu("ana@s.whatsapp.net".into())),
        ],
    );
    assert_eq!(tester.section.context_menu().unwrap().items.len(), 2);
    send(
        &mut tester,
        [Step::from(Message::RenameConversation(
            "ana@s.whatsapp.net".into(),
        ))],
    );
    assert!(tester.section.context_menu().is_none());
    assert_eq!(tester.section.alias(), Some(""));
    assert_eq!(
        tester.section.selected().map(|c| c.jid.as_str()),
        Some("ana@s.whatsapp.net")
    );
    send(
        &mut tester,
        [Step::from(BridgeEvent::Exited(
            "Não foi possível iniciar o whatsapp-bridge (arquivo não encontrado)".into(),
        ))],
    );
    let _ = tester.section.view();
}

#[test]
fn archived_conversations_are_hidden_until_toggled() {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(incoming(
                "1",
                "ana@s.whatsapp.net",
                "oi",
            ))),
            Step::from(BridgeEvent::Message(incoming("2", "123@g.us", "grupo"))),
            Step::from(BridgeEvent::Archived("123@g.us".into(), true)),
        ],
    );
    assert_eq!(tester.section.conversations().len(), 1);
    assert_eq!(tester.section.archived_count(), 1);
    let _ = tester.section.view();
    send(&mut tester, [Step::from(Message::ArchivedToggled)]);
    assert_eq!(tester.section.conversations()[0].jid, "123@g.us");
    let _ = tester.section.view();
    send(
        &mut tester,
        [
            Step::from(Message::ArchivedToggled),
            Step::from(Message::QueryChanged("@".into())),
        ],
    );
    assert_eq!(tester.section.conversations().len(), 2);
}

#[test]
fn images_download_only_when_requested() {
    let mut tester = section();
    let mut image = incoming("I1", "ana@s.whatsapp.net", "");
    image.tipo = "imagem".into();
    image.midia = "Zm9v".into();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(image)),
            Step::from(Message::Selected("ana@s.whatsapp.net".into())),
        ],
    );
    assert!(tester.section.media("I1").is_none());
    let _ = tester.section.view();
    send(
        &mut tester,
        [Step::from(Message::MediaRequested("I1".into()))],
    );
    assert!(matches!(tester.section.media("I1"), Some(Media::Failed(_))));
    send(
        &mut tester,
        [Step::from(Message::MediaLoaded(
            "I1".into(),
            Ok(std::path::PathBuf::from("foto.jpg")),
        ))],
    );
    assert!(matches!(tester.section.media("I1"), Some(Media::Ready(..))));
    let _ = tester.section.view();
    send(
        &mut tester,
        [Step::from(Message::MediaLoaded(
            "I1".into(),
            Err("expirou".into()),
        ))],
    );
    assert!(matches!(tester.section.media("I1"), Some(Media::Failed(e)) if e == "expirou"));
}
