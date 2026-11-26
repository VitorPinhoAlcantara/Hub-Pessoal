use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::whatsapp::engine::BridgeEvent;
use hub_core::whatsapp::protocol::WireConversation;
use hub_core::whatsapp::service::Processor;
use hub_core::whatsapp::{Message as ChatMessage, WireMessage};
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
        ..WireMessage::default()
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
fn exit_and_switch_off() {
    let mut tester = section();
    send(
        &mut tester,
        [Step::from(BridgeEvent::Exited("sem conexão".into()))],
    );
    assert!(
        matches!(tester.section.connection(), Connection::Offline(reason) if reason == "sem conexão")
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
    assert_eq!(tester.section.context_menu().unwrap().items.len(), 4);
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
fn right_click_offers_alias_and_failure_screen_renders() {
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
    assert_eq!(tester.section.context_menu().unwrap().items.len(), 3);
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
            "Conta suspensa temporariamente".into(),
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
fn dragging_reorders_pins() {
    let pins: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
    assert_eq!(reorder_pins(&pins, "a", "c").unwrap(), ["b", "c", "a"]);
    assert_eq!(reorder_pins(&pins, "c", "a").unwrap(), ["c", "a", "b"]);
    assert_eq!(reorder_pins(&pins, "x", "b").unwrap(), ["a", "x", "b", "c"]);
    assert_eq!(reorder_pins(&pins, "b", "x").unwrap(), ["a", "c"]);
    assert!(reorder_pins(&pins, "x", "y").is_none());
    assert!(reorder_pins(&pins, "a", "a").is_none());
}

#[test]
fn mentions_become_numbers_and_everyone_tags_all() {
    let mentions = vec![
        ("Ana".to_string(), "5511@s.whatsapp.net".to_string()),
        ("Ana Paula".to_string(), "77@lid".to_string()),
    ];
    let (text, jids) = apply_mentions("oi @Ana Paula e @Ana", &mentions, &[]);
    assert_eq!(text, "oi @77 e @5511");
    assert_eq!(jids, ["5511@s.whatsapp.net", "77@lid"]);
    let everyone = vec!["1@lid".to_string(), "2@lid".to_string()];
    let (text, jids) = apply_mentions("@todos reunião", &[], &everyone);
    assert_eq!(text, "@todos reunião");
    assert_eq!(jids, everyone);
}

const ANA: &str = "ana@s.whatsapp.net";
const BIA: &str = "bia@s.whatsapp.net";

fn two_chats() -> Tester {
    let mut tester = section();
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(incoming("1", ANA, "oi"))),
            Step::from(BridgeEvent::Message(incoming("2", BIA, "olá"))),
            Step::from(Message::Selected(ANA.into())),
        ],
    );
    tester
}

fn reply_ref() -> ReplyRef {
    ReplyRef {
        id: "1".into(),
        label: "Ana".into(),
        preview: "oi".into(),
    }
}

fn clock_now() -> hub_core::chrono::NaiveDateTime {
    hub_core::chrono::NaiveDate::from_ymd_opt(2026, 10, 21)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap()
}

fn mine_at(minutes_ago: i64, text: &str) -> ChatMessage {
    ChatMessage {
        id: "M".into(),
        text: text.into(),
        mine: true,
        downloadable: true,
        at: clock_now() - hub_core::chrono::TimeDelta::minutes(minutes_ago),
        ..ChatMessage::default()
    }
}

fn to_secs(local: &str) -> i64 {
    let at = hub_core::chrono::NaiveDateTime::parse_from_str(local, "%Y-%m-%d %H:%M").unwrap();
    hub_core::whatsapp::to_unix_millis(at) / 1000
}

#[test]
fn sending_requires_connection() {
    let mut tester = two_chats();
    send(
        &mut tester,
        [
            Step::from(Message::DraftChanged("tchau".into())),
            Step::from(Message::Send),
        ],
    );
    assert!(!tester.section.sending());
    assert_eq!(tester.section.draft(), "tchau");
    let out = Outgoing {
        jid: ANA.into(),
        text: "tchau".into(),
        ..Outgoing::default()
    };
    let wire = WireMessage {
        minha: true,
        id: "S1".into(),
        em: 1_792_627_100,
        ..incoming("S1", ANA, "tchau")
    };
    tester.section.draft_mut().unwrap().text.clear();
    send(
        &mut tester,
        [Step::from(Message::Sent(out, vec![Ok(wire)]))],
    );
    assert_eq!(tester.section.draft(), "");
    assert!(tester.section.messages().last().unwrap().mine);
}

#[test]
fn switching_chat_while_sending_keeps_each_draft() {
    let mut tester = two_chats();
    {
        let draft = tester.section.draft_mut().unwrap();
        draft.text = "para a Ana".into();
        draft.reply = Some(reply_ref());
    }
    let out = tester.section.prepare_send().unwrap();
    assert_eq!(out.jid, ANA);
    assert_eq!(out.reply, Some(reply_ref()));
    assert!(tester.section.sending());
    assert_eq!(tester.section.draft(), "");
    assert!(tester.section.prepare_send().is_none());
    send(
        &mut tester,
        [
            Step::from(Message::Selected(BIA.into())),
            Step::from(Message::DraftChanged("para a Bia".into())),
        ],
    );
    assert!(!tester.section.sending());
    send(
        &mut tester,
        [Step::from(Message::Sent(out, vec![Err("sem rede".into())]))],
    );
    assert_eq!(tester.section.draft(), "para a Bia");
    assert!(tester.section.error().unwrap().contains("sem rede"));
    send(&mut tester, [Step::from(Message::Selected(ANA.into()))]);
    assert_eq!(tester.section.draft(), "para a Ana");
    let draft = tester.section.current_draft().unwrap();
    assert_eq!(draft.reply, Some(reply_ref()));
    assert!(!tester.section.sending());
}

#[test]
fn failed_image_send_returns_image_and_reply() {
    let mut tester = two_chats();
    {
        let draft = tester.section.draft_mut().unwrap();
        draft.text = "legenda".into();
        draft.reply = Some(reply_ref());
        draft.image = Some(Pasted {
            png: std::sync::Arc::new(vec![1, 2, 3]),
            preview: None,
        });
    }
    let out = tester.section.prepare_send().unwrap();
    assert!(out.image.is_some());
    assert!(tester.section.current_draft().unwrap().is_empty());
    send(
        &mut tester,
        [Step::from(Message::Sent(out, vec![Err("falhou".into())]))],
    );
    let draft = tester.section.current_draft().unwrap();
    assert_eq!(draft.text, "legenda");
    assert!(draft.image.is_some());
    assert!(draft.reply.is_some());
}

#[test]
fn successful_send_keeps_draft_empty() {
    let mut tester = two_chats();
    tester.section.draft_mut().unwrap().text = "oi de novo".into();
    let out = tester.section.prepare_send().unwrap();
    let wire = WireMessage {
        minha: true,
        id: "S2".into(),
        em: 1_792_627_200,
        ..incoming("S2", ANA, "x")
    };
    send(
        &mut tester,
        [Step::from(Message::Sent(out, vec![Ok(wire)]))],
    );
    assert_eq!(tester.section.draft(), "");
    assert!(!tester.section.sending());
    assert_eq!(tester.section.messages().last().unwrap().text, "oi de novo");
}

#[test]
fn escape_closes_innermost_context_first() {
    let mut tester = two_chats();
    {
        let draft = tester.section.draft_mut().unwrap();
        draft.reply = Some(reply_ref());
        draft.edit = Some(EditRef {
            id: "1".into(),
            stash: "rascunho".into(),
        });
        draft.text = "editado".into();
    }
    send(
        &mut tester,
        [
            Step::from(Message::EmojiToggle),
            Step::from(Message::DrawerToggle),
            Step::from(Message::ForwardOpen("1".into())),
        ],
    );
    assert!(tester.section.dialog().is_some());
    assert!(tester.section.emoji_open);
    send(&mut tester, [Step::from(Message::Escape)]);
    assert!(tester.section.dialog().is_none());
    assert!(tester.section.emoji_open);
    send(&mut tester, [Step::from(Message::Escape)]);
    assert!(!tester.section.emoji_open);
    assert!(tester.section.current_draft().unwrap().edit.is_some());
    send(&mut tester, [Step::from(Message::Escape)]);
    let draft = tester.section.current_draft().unwrap();
    assert!(draft.edit.is_none());
    assert_eq!(draft.text, "rascunho");
    assert!(draft.reply.is_some());
    send(&mut tester, [Step::from(Message::Escape)]);
    assert!(tester.section.current_draft().unwrap().reply.is_none());
    assert!(tester.section.drawer.is_some());
    send(&mut tester, [Step::from(Message::Escape)]);
    assert!(tester.section.drawer.is_none());
}

#[test]
fn enter_on_empty_edit_does_nothing_and_with_text_edits() {
    let mut tester = two_chats();
    {
        let draft = tester.section.draft_mut().unwrap();
        draft.edit = Some(EditRef {
            id: "1".into(),
            stash: String::new(),
        });
        draft.text = "   ".into();
    }
    assert!(tester.section.prepare_send().is_none());
    assert!(tester.section.current_draft().unwrap().edit.is_some());
    tester.section.draft_mut().unwrap().text = "novo".into();
    let out = tester.section.prepare_send().unwrap();
    assert!(out.edit.is_some());
    assert_eq!(out.text, "novo");
}

#[test]
fn edit_is_limited_to_own_recent_text() {
    let now = clock_now();
    assert!(can_edit(&mine_at(5, "oi"), now));
    assert!(can_edit(&mine_at(15, "oi"), now));
    assert!(!can_edit(&mine_at(16, "oi"), now));
    let mut other = mine_at(1, "oi");
    other.mine = false;
    assert!(!can_edit(&other, now));
    let mut image = mine_at(1, "oi");
    image.kind = hub_core::whatsapp::MessageKind::Image;
    assert!(!can_edit(&image, now));
    let mut deleted = mine_at(1, "oi");
    deleted.deleted = true;
    assert!(!can_edit(&deleted, now));
    let mut legacy = mine_at(1, "oi");
    legacy.downloadable = false;
    assert!(!can_edit(&legacy, now));
}

#[test]
fn dangerous_extensions_are_detected() {
    for name in [
        "a.exe",
        "A.EXE",
        "x.msi",
        "x.bat",
        "x.cmd",
        "x.ps1",
        "x.vbs",
        "x.js",
        "x.lnk",
        "x.scr",
        "x.com",
        "x.jar",
        "x.hta",
        "x.reg",
        "x.sh",
        "x.desktop",
        "x.AppImage",
        "foto.pdf.exe",
        "c:\\pasta\\x.exe",
        "x.exe ",
        "x.exe.",
    ] {
        assert!(is_dangerous(name), "{name}");
    }
    for name in [
        "a.pdf",
        "foto.jpg",
        "sem_extensao",
        "exe",
        "a.exe.txt",
        "x.docx",
    ] {
        assert!(!is_dangerous(name), "{name}");
    }
}

#[test]
fn forward_destinations_toggle() {
    let mut selected = Vec::new();
    toggle_destination(&mut selected, "a");
    toggle_destination(&mut selected, "b");
    toggle_destination(&mut selected, "a");
    assert_eq!(selected, ["b"]);
    let mut tester = two_chats();
    send(
        &mut tester,
        [
            Step::from(Message::ForwardOpen("1".into())),
            Step::from(Message::ForwardToggle(ANA.into())),
            Step::from(Message::ForwardToggle(BIA.into())),
            Step::from(Message::ForwardToggle(ANA.into())),
        ],
    );
    let _ = tester.section.view();
    match tester.section.dialog() {
        Some(Dialog::Forward(state)) => assert_eq!(state.selected, [BIA]),
        _ => panic!("diálogo de encaminhar não abriu"),
    }
    send(
        &mut tester,
        [Step::from(Message::ForwardQuery("bia".into()))],
    );
    assert_eq!(tester.section.forward_targets("bia").len(), 1);
}

#[test]
fn message_window_limits_and_follows_target() {
    assert_eq!(visible_range(500, None, 120), 380..500);
    assert_eq!(visible_range(50, None, 120), 0..50);
    assert_eq!(visible_range(500, Some(200), 120), 80..200);
    assert_eq!(visible_range(10, Some(99), 5), 5..10);
}

#[test]
fn refresh_merge_keeps_older_loaded_messages() {
    let message = |id: &str| ChatMessage {
        id: id.into(),
        ..ChatMessage::default()
    };
    let current: Vec<ChatMessage> = ["a", "b", "c", "d", "e"].into_iter().map(message).collect();
    let fresh: Vec<ChatMessage> = ["c", "d", "e", "f"].into_iter().map(message).collect();
    let (merged, kept) = merge_messages(current.clone(), fresh);
    assert!(kept);
    let ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c", "d", "e", "f"]);
    let unrelated: Vec<ChatMessage> = ["x", "y"].into_iter().map(message).collect();
    let (merged, kept) = merge_messages(current, unrelated);
    assert!(!kept);
    assert_eq!(merged.len(), 2);
}

#[test]
fn media_is_fetched_only_when_requested_and_recovers() {
    let mut tester = section();
    let mut image = incoming("I1", ANA, "");
    image.tipo = "imagem".into();
    image.bruto = vec![1, 2, 3];
    image.tamanho = 50 * 1024 * 1024;
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(image)),
            Step::from(Message::Selected(ANA.into())),
        ],
    );
    assert!(tester.section.media("I1").is_none());
    let _ = tester.section.view();
    send(
        &mut tester,
        [Step::from(Message::MediaRequested("I1".into()))],
    );
    assert!(matches!(tester.section.media("I1"), Some(Media::Failed(_))));
    let file = MediaFile {
        path: std::path::PathBuf::from("foto.jpg"),
        preview: None,
    };
    send(
        &mut tester,
        [Step::from(Message::MediaLoaded("I1".into(), Ok(file)))],
    );
    assert!(matches!(
        tester.section.media("I1"),
        Some(Media::Ready(None, _))
    ));
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

#[test]
fn rich_message_kinds_render() {
    let mut tester = section();
    let kinds = [
        ("vid", "video"),
        ("aud", "audio"),
        ("doc", "documento"),
        ("fig", "figurinha"),
        ("con", "contato"),
        ("loc", "localizacao"),
        ("enq", "enquete"),
        ("img", "imagem"),
    ];
    for (id, kind) in kinds {
        let mut wire = incoming(id, ANA, "");
        wire.tipo = kind.into();
        wire.bruto = vec![1];
        wire.nome_arquivo = "relatorio.exe".into();
        wire.tamanho = 2048;
        wire.duracao = 12;
        send(&mut tester, [Step::from(BridgeEvent::Message(wire))]);
    }
    send(
        &mut tester,
        [
            Step::from(Message::Selected(ANA.into())),
            Step::from(Message::SearchToggle),
            Step::from(Message::DrawerToggle),
            Step::from(Message::EmojiToggle),
        ],
    );
    let _ = tester.section.view();
    send(
        &mut tester,
        [Step::from(Message::MessageMenu("doc".into()))],
    );
    assert!(tester.section.context_menu().is_some());
}

#[test]
fn message_menu_matches_ownership() {
    let mut tester = section();
    let mut mine = incoming("M1", ANA, "meu texto");
    mine.minha = true;
    mine.bruto = vec![1];
    mine.em = to_secs("2026-10-21 09:58");
    let mut theirs = incoming("T1", ANA, "dela");
    theirs.em = to_secs("2026-10-21 09:58");
    send(
        &mut tester,
        [
            Step::from(BridgeEvent::Message(mine)),
            Step::from(BridgeEvent::Message(theirs)),
            Step::from(Message::Selected(ANA.into())),
            Step::from(Message::MessageMenu("M1".into())),
        ],
    );
    let labels = |tester: &Tester| -> Vec<String> {
        tester
            .section
            .context_menu()
            .unwrap()
            .items
            .iter()
            .map(|i| i.label.clone())
            .collect()
    };
    let own = labels(&tester);
    assert!(own.contains(&"Editar".to_string()));
    assert!(own.contains(&"Apagar para todos".to_string()));
    send(&mut tester, [Step::from(Message::MessageMenu("T1".into()))]);
    let other = labels(&tester);
    assert!(!other.contains(&"Editar".to_string()));
    assert!(!other.contains(&"Apagar para todos".to_string()));
    assert!(other.contains(&"Apagar só aqui".to_string()));
    send(&mut tester, [Step::from(Message::EditStart("M1".into()))]);
    let draft = tester.section.current_draft().unwrap();
    assert_eq!(draft.text, "meu texto");
    assert_eq!(draft.edit.as_ref().unwrap().id, "M1");
}

#[test]
fn reactions_are_grouped_by_emoji() {
    let reaction = |emoji: &str, mine: bool| hub_core::whatsapp::Reaction {
        message_id: "1".into(),
        sender: "x".into(),
        emoji: emoji.into(),
        mine,
    };
    let groups = group_reactions(&[
        reaction("👍", false),
        reaction("❤️", false),
        reaction("👍", true),
    ]);
    assert_eq!(
        groups,
        [("👍".to_string(), 2, true), ("❤️".to_string(), 1, false)]
    );
}

#[test]
fn switching_chat_closes_dialogs() {
    let mut tester = two_chats();
    send(
        &mut tester,
        [
            Step::from(Message::ReactionPicker("1".into())),
            Step::from(Message::Selected(BIA.into())),
        ],
    );
    assert!(tester.section.dialog().is_none());
    assert_eq!(tester.section.selected().unwrap().jid, BIA);
}

#[test]
fn clicking_a_conversation_selects_it_on_release() {
    let mut tester = two_chats();
    send(
        &mut tester,
        [
            Step::from(Message::ConversationPressed(BIA.into())),
            Step::from(Message::DragReleased),
        ],
    );
    assert_eq!(tester.section.selected().unwrap().jid, BIA);
}
