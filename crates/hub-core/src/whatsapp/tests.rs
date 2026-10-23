use futures::StreamExt;

use super::bridge::{self, BridgeEvent};
use super::*;
use crate::Database;
use crate::platform::{Os, script_command_for};

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
        historico: false,
        midia: String::new(),
    }
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
    assert!(
        store_message(
            c,
            &wire_to_message(&wire("1", ana, "oi", false, 1_000, "texto"))
        )
        .unwrap()
    );
    assert!(
        !store_message(
            c,
            &wire_to_message(&wire("1", ana, "oi", false, 1_000, "texto"))
        )
        .unwrap()
    );
    store_message(
        c,
        &wire_to_message(&wire("2", group, "", false, 2_000, "imagem")),
    )
    .unwrap();
    store_message(
        c,
        &wire_to_message(&wire("3", ana, "tudo bem?", true, 3_000, "texto")),
    )
    .unwrap();
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
fn alias_has_priority_and_can_be_removed() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    store_message(
        c,
        &wire_to_message(&wire("1", jid, "oi", false, 1_000, "texto")),
    )
    .unwrap();
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
        store_message(
            db.conn(),
            &wire_to_message(&wire(
                &i.to_string(),
                jid,
                &format!("m{i}"),
                false,
                1_000 + i,
                "texto",
            )),
        )
        .unwrap();
    }
    let last = messages(db.conn(), jid, 3).unwrap();
    let texts: Vec<&str> = last.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, ["m7", "m8", "m9"]);
}

#[test]
fn display_text_combines_caption_and_kind() {
    let mut message = wire_to_message(&wire("1", "x", "legenda", false, 0, "video"));
    assert_eq!(message.display_text(), "[vídeo] legenda");
    message.kind = MessageKind::Text;
    assert_eq!(message.display_text(), "legenda");
}

#[test]
fn sidecar_command_points_to_data_dir() {
    let spec = sidecar_command(Path::new("dados"));
    let args: Vec<String> = spec
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args, ["--dados", "dados"]);
    assert!(spec.program.to_string_lossy().contains(SIDECAR_NAME));
}

fn fake_sidecar(dir: &Path) -> CommandSpec {
    let windows = r#"while ($true) {
  $line = [Console]::In.ReadLine()
  if ($null -eq $line) { break }
  $cmd = $line | ConvertFrom-Json
  if ($cmd.cmd -eq 'login') {
    Write-Output ('{"id":' + $cmd.id + ',"ok":true,"dados":null}')
    Write-Output '{"evento":"qr","codigo":"2@qr"}'
    Write-Output '{"evento":"conectado","jid":"eu@s.whatsapp.net"}'
    Write-Output '{"evento":"mensagem_recebida","mensagem":{"id":"M1","jid":"ana@s.whatsapp.net","de":"ana@s.whatsapp.net","nome_de":"Ana","texto":"oi","minha":false,"em":1792627000,"tipo":"texto"}}'
  } elseif ($cmd.cmd -eq 'listar_conversas') {
    Write-Output ('{"id":' + $cmd.id + ',"ok":true,"dados":[{"jid":"ana@s.whatsapp.net","nome":"Ana","grupo":false}]}')
  } elseif ($cmd.cmd -eq 'enviar') {
    Write-Output ('{"id":' + $cmd.id + ',"ok":true,"dados":{"id":"S1","jid":"' + $cmd.jid + '","de":"eu","nome_de":"","texto":"' + $cmd.texto + '","minha":true,"em":1792627100,"tipo":"texto"}}')
  } elseif ($cmd.cmd -eq 'encerrar') {
    Write-Output ('{"id":' + $cmd.id + ',"ok":true,"dados":null}')
    exit 0
  } else {
    Write-Output ('{"id":' + $cmd.id + ',"ok":false,"erro":"comando desconhecido"}')
  }
}
"#;
    let linux = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"login"'*)
      echo "{\"id\":$id,\"ok\":true,\"dados\":null}"
      echo '{"evento":"qr","codigo":"2@qr"}'
      echo '{"evento":"conectado","jid":"eu@s.whatsapp.net"}'
      echo '{"evento":"mensagem_recebida","mensagem":{"id":"M1","jid":"ana@s.whatsapp.net","de":"ana@s.whatsapp.net","nome_de":"Ana","texto":"oi","minha":false,"em":1792627000,"tipo":"texto"}}'
      ;;
    *'"listar_conversas"'*)
      echo "{\"id\":$id,\"ok\":true,\"dados\":[{\"jid\":\"ana@s.whatsapp.net\",\"nome\":\"Ana\",\"grupo\":false}]}"
      ;;
    *'"enviar"'*)
      echo "{\"id\":$id,\"ok\":true,\"dados\":{\"id\":\"S1\",\"jid\":\"ana@s.whatsapp.net\",\"de\":\"eu\",\"nome_de\":\"\",\"texto\":\"tchau\",\"minha\":true,\"em\":1792627100,\"tipo\":\"texto\"}}"
      ;;
    *'"encerrar"'*)
      echo "{\"id\":$id,\"ok\":true,\"dados\":null}"
      exit 0
      ;;
  esac
done
"#;
    let os = Os::current();
    let file = dir.join(if os == Os::Windows {
        "fake.ps1"
    } else {
        "fake.sh"
    });
    std::fs::write(&file, if os == Os::Windows { windows } else { linux }).unwrap();
    script_command_for(&file, os).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_talks_to_fake_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let command = fake_sidecar(temp.path());
    let mut events = Box::pin(bridge::spawn(command, temp.path().to_path_buf()));
    let Some(BridgeEvent::Ready(handle)) = events.next().await else {
        panic!("esperava pronto")
    };
    assert!(handle.send(protocol::Request::Login));
    let collector = tokio::spawn(async move {
        let mut seen = Vec::new();
        while let Some(event) = events.next().await {
            let exited = matches!(event, BridgeEvent::Exited(_));
            seen.push(event);
            if exited {
                break;
            }
        }
        seen
    });
    let conversations = handle.conversations().await.unwrap();
    assert_eq!(conversations[0].nome, "Ana");
    let sent = handle
        .send_text("ana@s.whatsapp.net", "tchau")
        .await
        .unwrap();
    assert!(sent.minha);
    assert_eq!(sent.texto, "tchau");
    drop(handle);
    let seen = tokio::time::timeout(std::time::Duration::from_secs(20), collector)
        .await
        .expect("o sidecar falso não terminou")
        .unwrap();
    assert!(
        seen.iter()
            .any(|e| matches!(e, BridgeEvent::Qr(code) if code == "2@qr"))
    );
    assert!(seen.iter().any(|e| matches!(e, BridgeEvent::Connected(_))));
    assert!(
        seen.iter()
            .any(|e| matches!(e, BridgeEvent::Message(m) if m.texto == "oi"))
    );
    assert!(matches!(seen.last(), Some(BridgeEvent::Exited(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_sidecar_reports_exit() {
    let temp = tempfile::tempdir().unwrap();
    let mut events = Box::pin(bridge::spawn(
        CommandSpec::new("whatsapp-bridge-que-nao-existe"),
        temp.path().to_path_buf(),
    ));
    assert!(
        matches!(events.next().await, Some(BridgeEvent::Exited(reason)) if reason.contains("sidecar"))
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
    store_message(
        c,
        &wire_to_message(&wire("1", lid, "oi", false, 1_792_627_000, "texto")),
    )
    .unwrap();
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
    store_message(c, &wire_to_message(&from_group)).unwrap();
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
                arquivada: false,
            },
            WireChatInfo {
                jid: "140419778236595@lid".into(),
                phone: "5579988887777@s.whatsapp.net".into(),
                nome: "Gabriel Jesus".into(),
                arquivada: false,
            },
            WireChatInfo {
                jid: group.into(),
                phone: String::new(),
                nome: String::new(),
                arquivada: true,
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
fn image_media_is_kept_and_filled_later() {
    let db = Database::open_in_memory().unwrap();
    let c = db.conn();
    let jid = "5511@s.whatsapp.net";
    let mut image = wire("I1", jid, "foto", false, 1_792_627_000, "imagem");
    store_message(c, &wire_to_message(&image)).unwrap();
    assert!(!messages(c, jid, 10).unwrap()[0].has_media());
    image.midia = "Zm9v".into();
    assert!(store_message(c, &wire_to_message(&image)).unwrap());
    let stored = &messages(c, jid, 10).unwrap()[0];
    assert!(stored.has_media());
    assert_eq!(stored.media, "Zm9v");
    assert!(!store_message(c, &wire_to_message(&image)).unwrap());
}
