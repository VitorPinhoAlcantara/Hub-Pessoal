use whatsapp_rust::prelude::{MessageBuilderExt, MessageField, wa};

use super::convert::{self, Content, Meta};
use super::engine::{self, NOT_CONNECTED};
use super::files;
use super::*;
use crate::Database;

fn wire(id: &str, jid: &str, text: &str, mine: bool, at: i64, kind: &str) -> WireMessage {
    WireMessage {
        id: id.into(),
        jid: jid.into(),
        de: if mine { "eu".into() } else { jid.into() },
        nome_de: "Ana".into(),
        texto: text.into(),
        minha: mine,
        em: at,
        tipo: kind.into(),
        ..WireMessage::default()
    }
}

fn put(conn: &Connection, wire: &WireMessage) -> bool {
    store_message(conn, &wire_to_message(wire)).unwrap()
}

#[test]
fn messages_are_stored_once_and_conversations_follow_last_message() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let ana = "5511@s.whatsapp.net";
    let group = "123@g.us";
    upsert_conversation(
        c,
        &WireConversation {
            jid: group.into(),
            nome: "Família".into(),
            grupo: true,
        },
    )
    .unwrap();
    assert!(put(c, &wire("1", ana, "oi", false, 1_000, "texto")));
    assert!(!put(c, &wire("1", ana, "oi", false, 1_000, "texto")));
    put(c, &wire("2", group, "", false, 2_000, "imagem"));
    put(c, &wire("3", ana, "tudo bem?", true, 3_000, "texto"));
    let list = conversations(c).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].jid, ana);
    assert_eq!(list[0].title(), "Ana");
    assert_eq!(list[0].preview.as_deref(), Some("tudo bem?"));
    assert_eq!(list[1].title(), "Família");
    assert!(list[1].group);
    assert_eq!(list[1].preview.as_deref(), Some("[imagem]"));
    let history = messages(c, ana, 10).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].text, "oi");
    assert!(history[1].mine);
    let image = &messages(c, group, 10).unwrap()[0];
    assert_eq!(image.display_text(), "[imagem]");
}

#[test]
fn pinned_conversations_come_first_in_their_order() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    for (id, jid, at) in [
        ("1", "a@s.whatsapp.net", 1_000),
        ("2", "b@s.whatsapp.net", 2_000),
        ("3", "c@s.whatsapp.net", 3_000),
    ] {
        put(c, &wire(id, jid, "oi", false, at, "texto"));
    }
    let order = |c| -> Vec<String> {
        conversations(c)
            .unwrap()
            .into_iter()
            .map(|x| x.jid)
            .collect()
    };
    assert_eq!(order(c)[0], "c@s.whatsapp.net");
    set_pinned_order(c, &["a@s.whatsapp.net".into(), "b@s.whatsapp.net".into()]).unwrap();
    assert_eq!(
        order(c),
        ["a@s.whatsapp.net", "b@s.whatsapp.net", "c@s.whatsapp.net"]
    );
    assert!(conversations(c).unwrap()[0].pinned);
    set_pinned_order(c, &["b@s.whatsapp.net".into()]).unwrap();
    assert_eq!(order(c)[0], "b@s.whatsapp.net");
    assert!(!conversations(c).unwrap()[1].pinned);
}

#[test]
fn alias_has_priority_and_can_be_removed() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    put(c, &wire("1", jid, "oi", false, 1_000, "texto"));
    set_alias(c, jid, "  Mãe ").unwrap();
    assert_eq!(conversations(c).unwrap()[0].title(), "Mãe");
    assert_eq!(find_conversation(c, "mãe").unwrap().unwrap().jid, jid);
    assert_eq!(find_conversation(c, jid).unwrap().unwrap().jid, jid);
    assert!(
        find_conversation(c, "999@s.whatsapp.net")
            .unwrap()
            .is_none()
    );
    set_alias(c, jid, "").unwrap();
    assert_eq!(conversations(c).unwrap()[0].title(), "Ana");
}

#[test]
fn messages_limit_keeps_latest_in_order() {
    let db = Database::open_in_memory().unwrap();
    let jid = "x@s.whatsapp.net";
    for i in 0..10 {
        put(
            db.conn(),
            &wire(
                &i.to_string(),
                jid,
                &format!("m{i}"),
                false,
                1_000 + i,
                "texto",
            ),
        );
    }
    let last = messages(db.conn(), jid, 3).unwrap();
    let texts: Vec<&str> = last.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["m7", "m8", "m9"]);
}

#[test]
fn messages_before_pages_backwards() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "x@s.whatsapp.net";
    for i in 0..10 {
        put(
            c,
            &wire(
                &i.to_string(),
                jid,
                &format!("m{i}"),
                false,
                1_000 + i,
                "texto",
            ),
        );
    }
    let anchor = messages(c, jid, 10).unwrap()[5].clone();
    let older = messages_before(c, jid, anchor.at, &anchor.id, 3).unwrap();
    let texts: Vec<&str> = older.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["m2", "m3", "m4"]);
    let oldest = messages(c, jid, 10).unwrap()[0].clone();
    assert!(
        messages_before(c, jid, oldest.at, &oldest.id, 3)
            .unwrap()
            .is_empty()
    );
    let (id, mine, _) = oldest_message(c, jid).unwrap().unwrap();
    assert_eq!(id, "0");
    assert!(!mine);
}

#[test]
fn display_text_combines_caption_and_kind() {
    let mut message = wire_to_message(&wire("1", "x", "legenda", false, 0, "video"));
    assert_eq!(message.display_text(), "[vídeo] legenda");
    message.kind = MessageKind::Text;
    assert_eq!(message.display_text(), "legenda");
    message.deleted = true;
    assert_eq!(message.display_text(), "[mensagem apagada]");
}

#[tokio::test]
async fn detached_handle_reports_not_connected() {
    let temp = tempfile::tempdir().unwrap();
    let handle = engine::detached(temp.path().to_path_buf(), temp.path().join("hub.db"));
    assert!(!handle.is_connected());
    assert_eq!(
        handle
            .send_text("a@s.whatsapp.net", "oi", Vec::new(), None)
            .await,
        Err(NOT_CONNECTED.to_string())
    );
    assert_eq!(handle.conversations().await, Err(NOT_CONNECTED.to_string()));
    assert_eq!(
        handle
            .request_history("a@s.whatsapp.net", HISTORY_BATCH)
            .await,
        Err(NOT_CONNECTED.to_string())
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), handle.shutdown())
            .await
            .is_ok()
    );
}

#[test]
fn link_holds_current_handle() {
    let link = WhatsAppLink::default();
    assert!(link.get().is_none());
    link.set(None);
    assert!(link.get().is_none());
}

#[test]
fn phone_labels_and_titles_hide_identifiers() {
    assert_eq!(
        phone_label("5579999998888@s.whatsapp.net").as_deref(),
        Some("+55 79 99999-8888")
    );
    assert_eq!(
        phone_label("447700900123@s.whatsapp.net").as_deref(),
        Some("+447700900123")
    );
    assert!(phone_label("140419778236595@lid").is_none());
    let mut conversation = Conversation {
        jid: "140419778236595@lid".into(),
        name: String::new(),
        alias: None,
        group: false,
        archived: false,
        last_at: None,
        preview: None,
        notify: false,
        pinned: false,
        photo: None,
    };
    assert_eq!(conversation.title(), "Contato sem nome");
    conversation.group = true;
    assert_eq!(conversation.title(), "Grupo sem nome");
    conversation.alias = Some("Chefe".into());
    assert_eq!(conversation.title(), "Chefe");
}

#[test]
fn chat_info_merges_lid_conversations_and_names_group_senders() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let lid = "269832025059549@lid";
    let phone = "5579999998888@s.whatsapp.net";
    let group = "123@g.us";
    put(c, &wire("1", lid, "oi", false, 1_792_627_000, "texto"));
    set_alias(c, lid, "Gabi").unwrap();
    let mut from_group = wire(
        "2",
        group,
        "tem 4 sem responsável",
        false,
        1_792_627_100,
        "texto",
    );
    from_group.de = "140419778236595@lid".into();
    from_group.nome_de = "Gabriel".into();
    put(c, &from_group);
    let mut known = known_jids(c).unwrap();
    known.sort();
    assert_eq!(known, ["123@g.us", "140419778236595@lid", lid]);
    apply_chat_info(
        c,
        &[
            WireChatInfo {
                jid: lid.into(),
                phone: phone.into(),
                nome: "Gabriela Souza".into(),
                arquivada: Some(false),
            },
            WireChatInfo {
                jid: "140419778236595@lid".into(),
                phone: "5579988887777@s.whatsapp.net".into(),
                nome: "Gabriel Jesus".into(),
                arquivada: Some(false),
            },
            WireChatInfo {
                jid: group.into(),
                phone: String::new(),
                nome: String::new(),
                arquivada: Some(true),
            },
        ],
    )
    .unwrap();
    let list = conversations(c).unwrap();
    assert!(list.iter().all(|conv| conv.jid != lid));
    let merged = list.iter().find(|conv| conv.jid == phone).unwrap();
    assert_eq!(merged.name, "Gabriela Souza");
    assert_eq!(merged.title(), "Gabi");
    assert_eq!(messages(c, phone, 10).unwrap().len(), 1);
    assert!(list.iter().find(|conv| conv.jid == group).unwrap().archived);
    let group_messages = messages(c, group, 10).unwrap();
    assert_eq!(group_messages[0].sender, "5579988887777@s.whatsapp.net");
    assert_eq!(group_messages[0].sender_label(), "Gabriel Jesus");
    set_alias(c, "5579988887777@s.whatsapp.net", "Gabi do grupo").unwrap();
    assert_eq!(
        messages(c, group, 10).unwrap()[0].sender_label(),
        "Gabi do grupo"
    );
    set_archived(c, group, false).unwrap();
    assert!(!conversations(c).unwrap().iter().any(|conv| conv.archived));
}

#[test]
fn chat_info_without_archive_state_keeps_it() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    put(c, &wire("1", jid, "oi", false, 1_000, "texto"));
    set_archived(c, jid, true).unwrap();
    apply_chat_info(
        c,
        &[WireChatInfo {
            jid: jid.into(),
            ..WireChatInfo::default()
        }],
    )
    .unwrap();
    assert!(conversation(c, jid).unwrap().unwrap().archived);
}

#[test]
fn merging_lid_chat_moves_reactions_and_photos() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let lid = "269832025059549@lid";
    let phone = "5579999998888@s.whatsapp.net";
    put(c, &wire("1", lid, "oi", false, 1_000, "texto"));
    apply_reaction(
        c,
        &WireReaction {
            jid: lid.into(),
            id: "1".into(),
            de: lid.into(),
            minha: false,
            emoji: "👍".into(),
            em: 1_001,
        },
    )
    .unwrap();
    set_photo(c, lid, "foto.jpg", from_unix(2_000)).unwrap();
    apply_chat_info(
        c,
        &[WireChatInfo {
            jid: lid.into(),
            phone: phone.into(),
            ..WireChatInfo::default()
        }],
    )
    .unwrap();
    assert!(reactions(c, lid).unwrap().is_empty());
    let moved = reactions(c, phone).unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].sender, phone);
    assert_eq!(moved[0].emoji, "👍");
    assert_eq!(
        photo(c, phone, from_unix(2_100)).unwrap(),
        Some(Some("foto.jpg".to_string()))
    );
    assert_eq!(photo(c, lid, from_unix(2_100)).unwrap(), None);
}

#[test]
fn media_is_marked_downloadable_and_filled_later() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    let mut image = wire("I1", jid, "foto", false, 1_792_627_000, "imagem");
    put(c, &image);
    assert!(!messages(c, jid, 10).unwrap()[0].has_media());
    image.bruto = vec![1, 2, 3];
    assert!(put(c, &image));
    let stored = &messages(c, jid, 10).unwrap()[0];
    assert!(stored.has_media());
    assert!(!put(c, &image));
    set_file(c, jid, "I1", "midia/I1.jpg").unwrap();
    assert_eq!(messages(c, jid, 10).unwrap()[0].file, "midia/I1.jpg");
    let raw = raw_message(c, jid, "I1").unwrap().unwrap();
    assert_eq!(raw.raw, vec![1, 2, 3]);
    assert_eq!(raw.kind, MessageKind::Image);
}

fn edit(jid: &str, id: &str, author: &str, mine: bool, text: &str) -> WireEdit {
    WireEdit {
        jid: jid.into(),
        id: id.into(),
        de: author.into(),
        minha: mine,
        texto: text.into(),
        em: 2_000,
    }
}

fn revoke(jid: &str, id: &str, author: &str, mine: bool, admin: bool) -> WireRevoke {
    WireRevoke {
        jid: jid.into(),
        id: id.into(),
        de: author.into(),
        minha: mine,
        admin,
        em: 2_000,
    }
}

fn from(author: &str, mut wire: WireMessage) -> WireMessage {
    wire.de = author.into();
    wire
}

#[test]
fn edits_and_revokes_apply_to_the_original_author_only() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "123@g.us";
    let ana = "5511@s.whatsapp.net";
    let beto = "5522@s.whatsapp.net";
    put(
        c,
        &from(ana, wire("1", jid, "versão 1", false, 1_000, "texto")),
    );
    assert_eq!(
        apply_edit(c, &edit(jid, "1", beto, false, "forjado")).unwrap(),
        Applied::Rejected
    );
    assert_eq!(messages(c, jid, 10).unwrap()[0].text, "versão 1");
    assert_eq!(
        apply_edit(c, &edit(jid, "1", ana, false, "versão 2")).unwrap(),
        Applied::Done
    );
    let current = &messages(c, jid, 10).unwrap()[0];
    assert_eq!(current.text, "versão 2");
    assert!(current.edited);
    assert_eq!(
        apply_revoke(c, &revoke(jid, "1", beto, false, false)).unwrap(),
        Applied::Rejected
    );
    assert!(!messages(c, jid, 10).unwrap()[0].deleted);
    assert_eq!(
        apply_revoke(c, &revoke(jid, "1", beto, false, true)).unwrap(),
        Applied::Done
    );
    let gone = &messages(c, jid, 10).unwrap()[0];
    assert!(gone.deleted);
    assert!(gone.text.is_empty());
    assert_eq!(gone.display_text(), "[mensagem apagada]");
    assert!(raw_message(c, jid, "1").unwrap().unwrap().raw.is_empty());
    assert_eq!(
        conversation(c, jid).unwrap().unwrap().preview.as_deref(),
        Some("[mensagem apagada]")
    );
}

#[test]
fn own_messages_accept_edits_from_another_device() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    put(c, &wire("1", jid, "oi", true, 1_000, "texto"));
    assert_eq!(
        apply_edit(
            c,
            &edit(jid, "1", "5599@s.whatsapp.net", true, "oi, tudo bem")
        )
        .unwrap(),
        Applied::Done
    );
    assert_eq!(messages(c, jid, 10).unwrap()[0].text, "oi, tudo bem");
    assert_eq!(
        apply_edit(c, &edit(jid, "1", jid, false, "forjado")).unwrap(),
        Applied::Rejected
    );
}

#[test]
fn edits_and_revokes_may_arrive_before_the_message() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "123@g.us";
    let ana = "5511@s.whatsapp.net";
    assert_eq!(
        apply_edit(c, &edit(jid, "1", ana, false, "editado")).unwrap(),
        Applied::Pending
    );
    assert_eq!(
        apply_revoke(c, &revoke(jid, "2", ana, false, false)).unwrap(),
        Applied::Pending
    );
    assert!(messages(c, jid, 10).unwrap().is_empty());
    assert!(conversations(c).unwrap().is_empty());
    put(
        c,
        &from(ana, wire("1", jid, "original", false, 1_000, "texto")),
    );
    put(
        c,
        &from(ana, wire("2", jid, "secreto", false, 1_001, "texto")),
    );
    let list = messages(c, jid, 10).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].text, "editado");
    assert!(list[0].edited);
    assert!(list[1].deleted);
    assert!(list[1].text.is_empty());
}

#[test]
fn pending_edits_from_a_different_author_are_discarded() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "123@g.us";
    apply_edit(c, &edit(jid, "1", "5599@s.whatsapp.net", false, "forjado")).unwrap();
    apply_revoke(c, &revoke(jid, "2", "5599@s.whatsapp.net", false, false)).unwrap();
    put(
        c,
        &from(
            "5511@s.whatsapp.net",
            wire("1", jid, "original", false, 1_000, "texto"),
        ),
    );
    put(
        c,
        &from(
            "5511@s.whatsapp.net",
            wire("2", jid, "intacta", false, 1_001, "texto"),
        ),
    );
    let list = messages(c, jid, 10).unwrap();
    assert_eq!(list[0].text, "original");
    assert!(!list[0].edited);
    assert_eq!(list[1].text, "intacta");
    assert!(!list[1].deleted);
}

#[test]
fn history_never_overwrites_an_applied_edit() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    put(c, &wire("1", jid, "versão 1", false, 1_000, "texto"));
    apply_edit(c, &edit(jid, "1", jid, false, "versão 2")).unwrap();
    let mut old_copy = wire("1", jid, "versão 1", false, 1_000, "texto");
    old_copy.historico = true;
    put(c, &old_copy);
    assert_eq!(messages(c, jid, 10).unwrap()[0].text, "versão 2");
    apply_revoke(c, &revoke(jid, "1", jid, false, false)).unwrap();
    put(c, &old_copy);
    let gone = &messages(c, jid, 10).unwrap()[0];
    assert!(gone.deleted);
    assert!(gone.text.is_empty());
}

#[test]
fn reactions_replace_remove_and_tolerate_any_order() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "123@g.us";
    let react = |from: &str, emoji: &str, mine: bool| WireReaction {
        jid: jid.into(),
        id: "1".into(),
        de: from.into(),
        minha: mine,
        emoji: emoji.into(),
        em: 3_000,
    };
    apply_reaction(c, &react("5511@s.whatsapp.net", "👍", false)).unwrap();
    put(
        c,
        &from(
            "5522@s.whatsapp.net",
            wire("1", jid, "oi", false, 1_000, "texto"),
        ),
    );
    apply_reaction(c, &react("5511@s.whatsapp.net", "❤️", false)).unwrap();
    apply_reaction(c, &react("eu", "😂", true)).unwrap();
    let list = reactions(c, jid).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].emoji, "❤️");
    assert!(list[1].mine);
    apply_reaction(c, &react("5511@s.whatsapp.net", "", false)).unwrap();
    assert_eq!(reactions(c, jid).unwrap().len(), 1);
    assert!(delete_local(c, jid, "1").unwrap());
    assert!(reactions(c, jid).unwrap().is_empty());
    assert!(messages(c, jid, 10).unwrap().is_empty());
}

#[test]
fn search_uses_literal_text_and_scopes() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let ana = "5511@s.whatsapp.net";
    let group = "123@g.us";
    put(c, &wire("1", ana, "100% certo", false, 1_000, "texto"));
    put(c, &wire("2", ana, "talvez", false, 1_001, "texto"));
    put(c, &wire("3", group, "CERTO então", false, 1_002, "texto"));
    let mut document = wire("4", group, "", false, 1_003, "documento");
    document.nome_arquivo = "relatório_final.pdf".into();
    put(c, &document);
    assert_eq!(search_messages(c, "100%", None, 10).unwrap().len(), 1);
    assert!(!search_messages(c, "%", None, 10).unwrap().is_empty());
    assert!(search_messages(c, "1_0", None, 10).unwrap().is_empty());
    let certo = search_messages(c, "certo", None, 10).unwrap();
    assert_eq!(certo.len(), 2);
    assert_eq!(certo[0].id, "3");
    assert_eq!(search_messages(c, "certo", Some(ana), 10).unwrap().len(), 1);
    assert_eq!(search_messages(c, "certo", None, 1).unwrap().len(), 1);
    assert_eq!(
        search_messages(c, "final.pdf", None, 10).unwrap()[0].id,
        "4"
    );
    assert!(search_messages(c, "   ", None, 10).unwrap().is_empty());
    apply_revoke(c, &revoke(ana, "1", ana, false, false)).unwrap();
    assert!(search_messages(c, "100%", None, 10).unwrap().is_empty());
}

#[test]
fn search_stays_fast_on_many_messages() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let tx = c.unchecked_transaction().unwrap();
    for i in 0..30_000 {
        let message = wire(
            &i.to_string(),
            "5511@s.whatsapp.net",
            &format!("mensagem número {i}"),
            false,
            1_000 + i,
            "texto",
        );
        store_message(&tx, &wire_to_message(&message)).unwrap();
    }
    tx.commit().unwrap();
    let started = std::time::Instant::now();
    let found = search_messages(c, "número 29999", None, 50).unwrap();
    assert_eq!(found.len(), 1);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn mention_names_come_from_contacts_and_replace_whole_tokens() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    upsert_conversation(
        c,
        &WireConversation {
            jid: "5511@s.whatsapp.net".into(),
            nome: "Ana Souza".into(),
            grupo: false,
        },
    )
    .unwrap();
    set_alias(c, "5522@s.whatsapp.net", "Beto").unwrap();
    let mentions = vec![
        WireMention {
            token: "5511".into(),
            jid: "5511@s.whatsapp.net".into(),
            name: String::new(),
        },
        WireMention {
            token: "5522".into(),
            jid: "5522@s.whatsapp.net".into(),
            name: "Roberto".into(),
        },
        WireMention {
            token: "77".into(),
            jid: "77@s.whatsapp.net".into(),
            name: "Carla".into(),
        },
        WireMention {
            token: "88".into(),
            jid: "88@s.whatsapp.net".into(),
            name: String::new(),
        },
    ];
    let text = resolve_mention_names(c, "oi @5511, @5522 e @77 @8899 @88", &mentions);
    assert_eq!(text, "oi @Ana Souza, @Beto e @Carla @8899 @88");
}

#[test]
fn photo_cache_expires_after_a_week() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    assert_eq!(photo(c, jid, from_unix(1_000)).unwrap(), None);
    set_photo(c, jid, "fotos/a.jpg", from_unix(1_000)).unwrap();
    assert_eq!(
        photo(c, jid, from_unix(1_000 + 6 * 86_400)).unwrap(),
        Some(Some("fotos/a.jpg".into()))
    );
    assert_eq!(photo(c, jid, from_unix(1_000 + 8 * 86_400)).unwrap(), None);
    set_photo(c, jid, "", from_unix(1_000)).unwrap();
    assert_eq!(photo(c, jid, from_unix(1_100)).unwrap(), Some(None));
    put(c, &wire("1", jid, "oi", false, 1_000, "texto"));
    set_photo(c, jid, "fotos/b.jpg", from_unix(1_000)).unwrap();
    assert_eq!(
        conversation(c, jid).unwrap().unwrap().photo.as_deref(),
        Some("fotos/b.jpg")
    );
}

fn meta(id: &str) -> Meta {
    Meta {
        id: id.into(),
        chat: "5511@s.whatsapp.net".into(),
        sender: "5511@s.whatsapp.net".into(),
        sender_raw: "5511@s.whatsapp.net".into(),
        push_name: "Ana".into(),
        ts: 1_792_627_000,
        ..Meta::default()
    }
}

fn message_of(content: Content) -> WireMessage {
    match content {
        Content::Message(wire) => *wire,
        other => panic!("esperava mensagem, veio {other:?}"),
    }
}

fn wrap_ephemeral(inner: wa::Message) -> wa::Message {
    let mut wrapper = wa::message::FutureProofMessage::default();
    wrapper.message = MessageField::some(inner);
    let mut message = wa::Message::default();
    message.ephemeral_message = MessageField::some(wrapper);
    message
}

fn protocol(kind: wa::message::protocol_message::Type, id: &str) -> wa::message::ProtocolMessage {
    let mut key = wa::MessageKey::default();
    key.id = Some(id.into());
    let mut protocol = wa::message::ProtocolMessage::default();
    protocol.key = MessageField::some(key);
    protocol.r#type = Some(kind);
    protocol
}

#[test]
fn text_messages_convert_with_quote_forward_and_mentions() {
    let mut context = wa::ContextInfo::default();
    context.stanza_id = Some("ORIG".into());
    context.participant = Some("5522@s.whatsapp.net".into());
    context.is_forwarded = Some(true);
    context.mentioned_jid = vec!["77@lid".into()];
    let mut quoted = wa::message::ImageMessage::default();
    quoted.caption = Some("praia".into());
    let mut quoted_message = wa::Message::default();
    quoted_message.image_message = MessageField::some(quoted);
    context.quoted_message = MessageField::some(quoted_message);
    let message = wa::Message::text_with_context("oi @77 e @99", context);
    let linked = convert::links(&message);
    assert_eq!(linked.quoted_id, "ORIG");
    assert_eq!(linked.quoted_text, "[imagem] praia");
    assert_eq!(linked.mentioned, ["77@lid"]);
    let mut info = meta("M1");
    info.quoted_from = "5522@s.whatsapp.net".into();
    info.own_tokens = vec!["99".into()];
    info.mentions = vec![WireMention {
        token: "77".into(),
        jid: "5533@s.whatsapp.net".into(),
        name: "Carla".into(),
    }];
    let wire = message_of(convert::build(&info, &message));
    assert_eq!(wire.tipo, "texto");
    assert_eq!(wire.texto, "oi @77 e @você");
    assert_eq!(wire.citada_id, "ORIG");
    assert_eq!(wire.citada_de, "5522@s.whatsapp.net");
    assert!(wire.encaminhada);
    assert_eq!(wire.nome_de, "Ana");
    assert_eq!(wire.mencoes.len(), 1);
    assert!(!wire.bruto.is_empty());
    assert_eq!(wire.autor_bruto, "5511@s.whatsapp.net");
    let mut mine = meta("M2");
    mine.from_me = true;
    assert_eq!(
        message_of(convert::build(&mine, &wa::Message::text("eu"))).nome_de,
        ""
    );
}

#[test]
fn media_messages_keep_their_details() {
    let mut image = wa::message::ImageMessage::default();
    image.caption = Some("foto".into());
    image.mimetype = Some("image/jpeg".into());
    image.file_length = Some(2048);
    image.width = Some(800);
    image.height = Some(600);
    let mut message = wa::Message::default();
    message.image_message = MessageField::some(image);
    let wire = message_of(convert::build(&meta("I"), &wrap_ephemeral(message)));
    assert_eq!(wire.tipo, "imagem");
    assert_eq!(wire.texto, "foto");
    assert_eq!((wire.largura, wire.altura, wire.tamanho), (800, 600, 2048));
    assert_eq!(wire.mime, "image/jpeg");

    let mut audio = wa::message::AudioMessage::default();
    audio.ptt = Some(true);
    audio.seconds = Some(7);
    audio.mimetype = Some("audio/ogg; codecs=opus".into());
    let mut message = wa::Message::default();
    message.audio_message = MessageField::some(audio);
    let wire = message_of(convert::build(&meta("A"), &message));
    assert_eq!(wire.tipo, "audio");
    assert!(wire.voz);
    assert_eq!(wire.duracao, 7);

    let mut video = wa::message::VideoMessage::default();
    video.seconds = Some(12);
    video.caption = Some("clipe".into());
    let mut message = wa::Message::default();
    message.video_message = MessageField::some(video);
    let wire = message_of(convert::build(&meta("V"), &message));
    assert_eq!(
        (wire.tipo.as_str(), wire.duracao, wire.texto.as_str()),
        ("video", 12, "clipe")
    );

    let mut document = wa::message::DocumentMessage::default();
    document.file_name = Some("contrato.pdf".into());
    document.mimetype = Some("application/pdf".into());
    let mut caption = wa::message::FutureProofMessage::default();
    let mut inner = wa::Message::default();
    inner.document_message = MessageField::some(document);
    caption.message = MessageField::some(inner);
    let mut message = wa::Message::default();
    message.document_with_caption_message = MessageField::some(caption);
    let wire = message_of(convert::build(&meta("D"), &message));
    assert_eq!(wire.tipo, "documento");
    assert_eq!(wire.nome_arquivo, "contrato.pdf");
    assert_eq!(wire.texto, "contrato.pdf");

    let mut sticker = wa::message::StickerMessage::default();
    sticker.width = Some(512);
    let mut message = wa::Message::default();
    message.sticker_message = MessageField::some(sticker);
    assert_eq!(
        message_of(convert::build(&meta("S"), &message)).tipo,
        "figurinha"
    );
}

#[test]
fn contacts_locations_and_polls_get_indicative_text() {
    let mut contact = wa::message::ContactMessage::default();
    contact.display_name = Some("Dona Maria".into());
    let mut message = wa::Message::default();
    message.contact_message = MessageField::some(contact);
    let wire = message_of(convert::build(&meta("C"), &message));
    assert_eq!(
        (wire.tipo.as_str(), wire.texto.as_str()),
        ("contato", "Dona Maria")
    );

    let mut place = wa::message::LocationMessage::default();
    place.degrees_latitude = Some(-9.665);
    place.degrees_longitude = Some(-35.735);
    let mut message = wa::Message::default();
    message.location_message = MessageField::some(place);
    let wire = message_of(convert::build(&meta("L"), &message));
    assert_eq!(wire.tipo, "localizacao");
    assert_eq!(wire.texto, "-9.665000, -35.735000");

    let mut poll = wa::message::PollCreationMessage::default();
    poll.name = Some("Almoço?".into());
    for name in ["Sim", "Não"] {
        let mut option = wa::message::poll_creation_message::Option::default();
        option.option_name = Some(name.into());
        poll.options.push(option);
    }
    let mut message = wa::Message::default();
    message.poll_creation_message = MessageField::some(poll);
    let wire = message_of(convert::build(&meta("P"), &message));
    assert_eq!(wire.tipo, "enquete");
    assert_eq!(wire.texto, "Almoço?\n1. Sim\n2. Não");
    assert_eq!(
        wire_to_message(&wire).display_text(),
        "[enquete] Almoço?\n1. Sim\n2. Não"
    );
}

#[test]
fn protocol_messages_become_edits_revokes_and_reactions() {
    use wa::message::protocol_message::Type;
    let mut edit = protocol(Type::MESSAGE_EDIT, "E1");
    edit.edited_message = MessageField::some(wa::Message::text("corrigido"));
    let mut message = wa::Message::default();
    message.protocol_message = MessageField::some(edit);
    match convert::build(&meta("X"), &message) {
        Content::Edit(edit) => {
            assert_eq!((edit.id.as_str(), edit.texto.as_str()), ("E1", "corrigido"));
            assert_eq!(edit.de, "5511@s.whatsapp.net");
            assert!(!edit.minha);
        }
        other => panic!("esperava edição, veio {other:?}"),
    }
    let mut message = wa::Message::default();
    message.protocol_message = MessageField::some(protocol(Type::REVOKE, "R1"));
    let mut admin = meta("X");
    admin.admin_revoke = true;
    match convert::build(&admin, &message) {
        Content::Revoke(revoke) => {
            assert_eq!(revoke.id, "R1");
            assert!(revoke.admin);
        }
        other => panic!("esperava revogação, veio {other:?}"),
    }
    let mut message = wa::Message::default();
    message.protocol_message = MessageField::some(protocol(Type::EPHEMERAL_SETTING, "Z"));
    assert_eq!(convert::build(&meta("X"), &message), Content::Skip);

    let mut key = wa::MessageKey::default();
    key.id = Some("T1".into());
    let mut reaction = wa::message::ReactionMessage::default();
    reaction.key = MessageField::some(key);
    reaction.text = Some("🔥".into());
    let mut message = wa::Message::default();
    message.reaction_message = MessageField::some(reaction);
    match convert::build(&meta("X"), &wrap_ephemeral(message)) {
        Content::Reaction(reaction) => {
            assert_eq!(
                (reaction.id.as_str(), reaction.emoji.as_str()),
                ("T1", "🔥")
            );
        }
        other => panic!("esperava reação, veio {other:?}"),
    }
}

#[test]
fn protocol_noise_is_skipped_but_unknown_content_is_kept() {
    let mut skdm = wa::Message::default();
    skdm.sender_key_distribution_message =
        MessageField::some(wa::message::SenderKeyDistributionMessage::default());
    assert_eq!(convert::build(&meta("N"), &skdm), Content::Skip);
    assert_eq!(
        convert::build(&meta("N"), &wa::Message::default()),
        Content::Skip
    );
    let mut invite = wa::Message::default();
    invite.group_invite_message = MessageField::some({
        let mut invite = wa::message::GroupInviteMessage::default();
        invite.group_name = Some("Grupo".into());
        invite
    });
    let wire = message_of(convert::build(&meta("G"), &invite));
    assert_eq!(wire.tipo, "outro");
    assert_eq!(
        wire_to_message(&wire).display_text(),
        "[mensagem não suportada]"
    );
}

#[test]
fn converted_messages_flow_into_the_database() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let wire = message_of(convert::build(&meta("M1"), &wa::Message::text("olá")));
    assert!(put(c, &wire));
    let stored = &messages(c, "5511@s.whatsapp.net", 10).unwrap()[0];
    assert_eq!(stored.text, "olá");
    assert_eq!(stored.sender_raw, "5511@s.whatsapp.net");
    assert!(stored.downloadable);
    let raw = raw_message(c, "5511@s.whatsapp.net", "M1")
        .unwrap()
        .unwrap();
    assert!(!raw.raw.is_empty());
}

#[test]
fn file_names_are_sanitized() {
    assert_eq!(files::safe_file_name("../../etc/passwd", ""), "passwd");
    assert_eq!(
        files::safe_file_name("C:\\Windows\\system32\\evil.exe", ""),
        "evil.bin"
    );
    assert_eq!(files::safe_file_name("script.ps1", "pdf"), "script.bin");
    assert_eq!(files::safe_file_name("CON.txt", ""), "_CON.txt");
    assert_eq!(
        files::safe_file_name("nome:inválido?.pdf", ""),
        "nomeinválido.pdf"
    );
    assert_eq!(files::safe_file_name("  ..  ", "jpg"), "arquivo.jpg");
    assert_eq!(
        files::safe_file_name("sem_extensao", "pdf"),
        "sem_extensao.pdf"
    );
    assert_eq!(
        files::safe_file_name("relatório final.PDF", ""),
        "relatório final.pdf"
    );
    assert!(files::safe_file_name(&"a".repeat(500), "").len() <= 90);
    assert_eq!(files::sanitize_id("../3EB0/ABC"), "3EB0ABC");
    assert_eq!(files::sanitize_id("///"), "midia");
    let name = files::media_file_name(
        "../X1",
        MessageKind::Document,
        "application/pdf",
        "..\\..\\a.exe",
    );
    assert_eq!(name, "X1_a.bin");
    assert_eq!(
        files::media_file_name("X1", MessageKind::Image, "image/jpeg", ""),
        "X1.jpg"
    );
    let dir = std::path::Path::new("dados/midia");
    assert!(files::is_inside(
        dir,
        &files::media_path(dir, "../../x", MessageKind::Image, "image/png", "")
    ));
    assert_eq!(
        files::photo_file_name("123@s.whatsapp.net"),
        "123_s_whatsapp_net.jpg"
    );
}

#[test]
fn only_whatsapp_hosts_are_trusted_for_pictures() {
    assert!(files::is_trusted_picture_url(
        "https://pps.whatsapp.net/v/t61/a.jpg?x=1"
    ));
    assert!(files::is_trusted_picture_url(
        "https://scontent.xx.fbcdn.net/a.jpg"
    ));
    assert!(!files::is_trusted_picture_url(
        "http://pps.whatsapp.net/a.jpg"
    ));
    assert!(!files::is_trusted_picture_url(
        "https://evil.com/.whatsapp.net"
    ));
    assert!(!files::is_trusted_picture_url(
        "https://pps.whatsapp.net@evil.com/a.jpg"
    ));
    assert!(!files::is_trusted_picture_url(
        "https://whatsapp.net.evil.com/a.jpg"
    ));
    assert!(!files::is_trusted_picture_url("file:///c:/segredo"));
}

#[test]
fn outgoing_files_are_classified_by_extension() {
    use files::OutKind::*;
    let kind = |name: &str| files::classify_path(std::path::Path::new(name)).0;
    assert_eq!(kind("a.JPG"), Image);
    assert_eq!(kind("a.mp4"), Video);
    assert_eq!(kind("a.mp3"), Audio);
    assert_eq!(kind("a.pdf"), Document);
    assert_eq!(kind("a.gif"), Document);
    assert_eq!(kind("semextensao"), Document);
    assert_eq!(
        files::classify_path(std::path::Path::new("a.ogg")).1,
        "audio/ogg; codecs=opus"
    );
}

#[test]
fn ban_messages_are_clear_and_in_portuguese() {
    use std::time::Duration;
    use whatsapp_rust::types::events::TempBanReason;
    assert_eq!(engine::human_duration(Duration::from_secs(30)), "1 minuto");
    assert_eq!(
        engine::human_duration(Duration::from_secs(25 * 60)),
        "25 minutos"
    );
    assert_eq!(
        engine::human_duration(Duration::from_secs(3 * 3600)),
        "3 horas"
    );
    assert_eq!(
        engine::human_duration(Duration::from_secs(5 * 86_400)),
        "5 dias"
    );
    let text = engine::ban_message(&TempBanReason::BlockedByUsers, Duration::from_secs(7_200));
    assert!(text.contains("suspendeu temporariamente"));
    assert!(text.contains("2 horas"));
    assert!(text.contains("não vai tentar reconectar"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_store_is_created_and_reset_without_network() {
    use whatsapp_rust::prelude::Bot;
    use whatsapp_rust::store::{SqliteDatabase, SqliteDatabaseConfig};
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("pasta com espaço");
    std::fs::create_dir_all(&dir).unwrap();
    let url = dir
        .join(engine::SESSION_FILE)
        .to_string_lossy()
        .into_owned();
    let db = SqliteDatabase::open(&url, SqliteDatabaseConfig::default())
        .await
        .unwrap();
    assert!(!db.device_exists(1).await.unwrap());
    let bot = Bot::builder()
        .with_backend(db.store(1))
        .build()
        .await
        .unwrap();
    assert!(bot.client().pn().is_none());
    assert!(!bot.client().is_logged_in());
    assert!(db.device_exists(1).await.unwrap());
    drop(bot);
    db.reset_device(1).await.unwrap();
    assert!(db.device_exists(1).await.unwrap());
    assert!(dir.join(engine::SESSION_FILE).is_file());
}

#[test]
fn history_queue_puts_focus_first_then_pinned_then_recent() {
    let mut queue = super::service::HistoryQueue::default();
    for (jid, base) in [("a", 100), ("b", 300), ("c", 200), ("d", 400)] {
        for index in 0..60 {
            queue.push(WireMessage {
                id: format!("{jid}{index}"),
                jid: jid.to_string(),
                em: base + index,
                ..WireMessage::default()
            });
        }
    }
    let pinned = vec!["c".to_string()];
    let first = queue.next_page(Some("a"), &pinned);
    assert!(first.iter().all(|m| m.jid == "a"));
    assert_eq!(first.len(), 50);
    assert_eq!(first.first().unwrap().em, 110);
    let order: Vec<String> = (0..7)
        .map(|_| queue.next_page(None, &pinned)[0].jid.clone())
        .collect();
    assert_eq!(order, ["c", "d", "b", "c", "d", "b", "a"]);
    assert!(queue.is_empty());
}

fn sticker_raw(seed: u8) -> Vec<u8> {
    let mut sticker = wa::message::StickerMessage::default();
    sticker.file_sha256 = Some(vec![seed; 32]);
    sticker.direct_path = Some("/v/figurinha".into());
    let mut message = wa::Message::default();
    message.sticker_message = MessageField::some(sticker);
    whatsapp_rust::waproto::codec::message_to_vec(&message)
}

#[test]
fn stickers_are_split_by_section_and_unique() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    for (id, seed, mine, at) in [
        ("a", 1, false, 1),
        ("b", 1, false, 2),
        ("c", 2, false, 3),
        ("d", 3, true, 4),
    ] {
        let mut sticker = wire(
            id,
            "5511@s.whatsapp.net",
            "",
            mine,
            1_792_000_000 + at,
            "figurinha",
        );
        sticker.bruto = sticker_raw(seed);
        put(c, &sticker);
    }
    let received = stickers(c, StickerSection::Received, 10).unwrap();
    assert_eq!(received.len(), 2);
    assert_eq!(stickers(c, StickerSection::Recent, 10).unwrap().len(), 1);
    let first = &received[0];
    set_favorite_sticker(c, &first.hash, &first.action, true, 1).unwrap();
    assert!(favorite_hashes(c).unwrap().contains(&first.hash));
    assert_eq!(
        stickers(c, StickerSection::Favorite, 10).unwrap()[0].action,
        first.action
    );
    set_favorite_sticker(c, &first.hash, &first.action, false, 2).unwrap();
    assert!(
        stickers(c, StickerSection::Favorite, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sticker_image_becomes_square_webp() {
    let mut png = Vec::new();
    image::DynamicImage::new_rgba8(300, 100)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let webp = engine::sticker_webp(&png).unwrap();
    let decoded = image::load_from_memory(&webp).unwrap();
    assert_eq!(
        (decoded.width(), decoded.height()),
        (engine::STICKER_SIZE, engine::STICKER_SIZE)
    );
}
