use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::stream::BoxStream;
use hub_core::clock::FakeClock;
use hub_core::platform::{FakePlatform, Os, script_command_for};
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

struct Canned;

impl AiProvider for Canned {
    fn label(&self) -> &'static str {
        "Claude CLI"
    }

    fn send(&self, _request: AiRequest) -> BoxStream<'static, AiEvent> {
        futures::stream::iter(vec![AiEvent::Text("ok".into())]).boxed()
    }
}

fn chat() -> Chat {
    let ctx = HubContext::new(
        HubPaths::from_home("teste"),
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 14:00")),
    );
    Chat::new(ctx, Arc::new(Canned))
}

fn send(chat: &mut Chat, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = chat.update(message);
    }
}

#[test]
fn sending_adds_user_item_and_marks_running() {
    let mut chat = chat();
    send(&mut chat, [Message::Send]);
    assert!(chat.items().is_empty());
    send(
        &mut chat,
        [Message::InputChanged(" oi ".into()), Message::Send],
    );
    assert!(matches!(&chat.items()[0], Item::User { text, .. } if text == "oi"));
    assert!(chat.is_running());
    assert_eq!(chat.input(), "");
    send(
        &mut chat,
        [Message::InputChanged("outra".into()), Message::Send],
    );
    assert_eq!(chat.items().len(), 1);
    send(&mut chat, [Message::Finished]);
    assert!(!chat.is_running());
}

#[test]
fn events_build_streaming_text_and_tool_cards() {
    let mut chat = chat();
    send(
        &mut chat,
        [
            Message::Event(AiEvent::Session("s1".into())),
            Message::Event(AiEvent::TextDelta("Vou ".into())),
            Message::Event(AiEvent::TextDelta("criar".into())),
            Message::Event(AiEvent::ToolUse {
                id: "t1".into(),
                name: "mcp__hub__lembrete_criar".into(),
                input: "{}".into(),
            }),
            Message::Event(AiEvent::ToolResult {
                id: "t1".into(),
                output: "criado".into(),
                is_error: false,
            }),
            Message::Event(AiEvent::Text("Pronto".into())),
            Message::Event(AiEvent::Done {
                session: Some("s1".into()),
                is_error: false,
                summary: None,
            }),
        ],
    );
    assert_eq!(chat.session(), Some("s1"));
    assert!(matches!(&chat.items()[0], Item::Assistant { raw, .. } if raw == "Vou criar"));
    assert!(matches!(&chat.items()[1], Item::Tool { output: Some(o), .. } if o == "criado"));
    assert!(matches!(&chat.items()[2], Item::Assistant { raw, .. } if raw == "Pronto"));
    assert_eq!(
        short_tool_name("mcp__hub__lembrete_criar"),
        "lembrete_criar"
    );
}

#[test]
fn cancel_and_new_conversation() {
    let mut chat = chat();
    send(
        &mut chat,
        [
            Message::InputChanged("oi".into()),
            Message::Send,
            Message::Cancel,
        ],
    );
    assert!(!chat.is_running());
    assert!(matches!(chat.items().last(), Some(Item::Notice(_))));
    send(
        &mut chat,
        [
            Message::Event(AiEvent::Session("x".into())),
            Message::NewConversation,
        ],
    );
    assert!(chat.items().is_empty());
    assert!(chat.session().is_none());
}

#[test]
fn errors_are_shown() {
    let mut chat = chat();
    send(
        &mut chat,
        [
            Message::Event(AiEvent::Error("sem login".into())),
            Message::Event(AiEvent::Done {
                session: None,
                is_error: true,
                summary: Some("limite".into()),
            }),
        ],
    );
    assert!(matches!(&chat.items()[0], Item::Error(e) if e == "sem login"));
    assert!(matches!(&chat.items()[1], Item::Error(e) if e == "limite"));
}

#[test]
fn view_builds_with_every_item() {
    let mut chat = chat();
    let _ = chat.view();
    send(
        &mut chat,
        [
            Message::InputChanged("oi".into()),
            Message::Send,
            Message::Event(AiEvent::TextDelta("```rust\nfn main() {}\n```".into())),
            Message::Event(AiEvent::ToolUse {
                id: "t".into(),
                name: "x".into(),
                input: "{}".into(),
            }),
            Message::Event(AiEvent::Error("e".into())),
        ],
    );
    let _ = chat.view();
}

#[test]
fn text_and_pdf_files_become_attachments() {
    let text = Attachment::from_bytes("notas.md".into(), b"# oi".to_vec()).unwrap();
    assert!(matches!(&text.kind, AttachmentKind::Text { content } if &**content == "# oi"));
    assert!(text.preview().is_none());
    let pdf = Attachment::from_bytes("doc.PDF".into(), b"%PDF-1.7".to_vec()).unwrap();
    assert!(matches!(pdf.kind, AttachmentKind::Pdf { .. }));
    assert!(Attachment::from_bytes("app.exe".into(), vec![0x4d, 0x5a, 0, 0xff]).is_err());
    assert!(Attachment::from_bytes("grande.txt".into(), vec![b'a'; MAX_TEXT_BYTES + 1]).is_err());
    let mut chat = chat();
    send(
        &mut chat,
        [
            Message::AttachmentLoaded(Ok(text)),
            Message::AttachmentLoaded(Ok(pdf)),
        ],
    );
    assert_eq!(chat.attachments().len(), 2);
    let _ = chat.view();
    send(
        &mut chat,
        [Message::ClipboardRead(ClipboardContent {
            files: Vec::new(),
            png: Some(PNG.to_vec()),
        })],
    );
    assert_eq!(chat.attachments().len(), 3);
    assert_eq!(chat.attachments()[2].name, "imagem colada.png");
}

fn fake_cli(temp: &tempfile::TempDir, windows: &str, linux: &str) -> ClaudeCli {
    let os = Os::current();
    let file = match os {
        Os::Windows => temp.path().join("fake.ps1"),
        Os::Linux => temp.path().join("fake.sh"),
    };
    std::fs::write(&file, if os == Os::Windows { windows } else { linux }).unwrap();
    let mut cli = ClaudeCli::new(temp.path().to_path_buf(), Some(PathBuf::from("mcp.json")));
    cli.command = script_command_for(&file, os).unwrap();
    cli
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_provider_streams_events_from_process() {
    let temp = tempfile::tempdir().unwrap();
    let windows = r#"$prompt = [Console]::In.ReadToEnd()
Write-Output '{"type":"system","subtype":"init","session_id":"fake-1"}'
$word = if ($prompt.Contains('"text":"teste"')) { 'teste' } else { 'vazio' }
Write-Output ('{"type":"assistant","message":{"content":[{"type":"text","text":"eco: ' + $word + '"}]}}')
Write-Output '{"type":"result","subtype":"success","is_error":false,"session_id":"fake-1","result":"ok"}'
"#;
    let linux = r#"prompt=$(cat)
echo '{"type":"system","subtype":"init","session_id":"fake-1"}'
case "$prompt" in *'"text":"teste"'*) word=teste;; *) word=vazio;; esac
echo "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"eco: $word\"}]}}"
echo '{"type":"result","subtype":"success","is_error":false,"session_id":"fake-1","result":"ok"}'
"#;
    let cli = fake_cli(&temp, windows, linux);
    let events: Vec<AiEvent> = cli
        .send(AiRequest {
            prompt: "teste".into(),
            attachments: Vec::new(),
            session: None,
        })
        .collect()
        .await;
    assert_eq!(events[0], AiEvent::Session("fake-1".into()));
    assert_eq!(events[1], AiEvent::Text("eco: teste".into()));
    assert!(matches!(events[2], AiEvent::Done { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_provider_reports_failure_without_result() {
    let temp = tempfile::tempdir().unwrap();
    let cli = fake_cli(
        &temp,
        "[Console]::Error.WriteLine('não logado')\nexit 1\n",
        "echo 'não logado' 1>&2\nexit 1\n",
    );
    let events: Vec<AiEvent> = cli
        .send(AiRequest {
            prompt: "x".into(),
            attachments: Vec::new(),
            session: None,
        })
        .collect()
        .await;
    assert!(matches!(events.last(), Some(AiEvent::Error(e)) if e.contains("logado")));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_program_is_reported() {
    let mut cli = ClaudeCli::new(PathBuf::from("."), None);
    cli.command = hub_core::platform::CommandSpec::new("programa-que-nao-existe-hub");
    let events: Vec<AiEvent> = cli
        .send(AiRequest {
            prompt: "x".into(),
            attachments: Vec::new(),
            session: None,
        })
        .collect()
        .await;
    assert!(matches!(events.as_slice(), [AiEvent::Error(e)] if e.contains("Claude CLI")));
}

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
];

#[test]
fn cancelled_message_can_be_edited_and_resent() {
    let mut chat = chat();
    send(
        &mut chat,
        [
            Message::InputChanged("primeira\nlinha".into()),
            Message::Send,
            Message::Event(AiEvent::TextDelta("parcial".into())),
            Message::Cancel,
        ],
    );
    assert_eq!(chat.cancelled(), Some(0));
    send(&mut chat, [Message::EditResend(0)]);
    assert!(chat.items().is_empty());
    assert_eq!(chat.input().trim_end(), "primeira\nlinha");
    assert!(chat.cancelled().is_none());
    send(&mut chat, [Message::Send]);
    assert!(matches!(&chat.items()[0], Item::User { text, .. } if text == "primeira\nlinha"));
}

#[test]
fn edit_is_ignored_while_running_and_menu_offers_actions() {
    let mut chat = chat();
    send(
        &mut chat,
        [Message::InputChanged("oi".into()), Message::Send],
    );
    send(&mut chat, [Message::EditResend(0)]);
    assert_eq!(chat.items().len(), 1);
    send(&mut chat, [Message::ItemMenu(0)]);
    let menu = chat.context_menu().unwrap();
    assert_eq!(menu.items.len(), 2);
    assert!(menu.items[1].message.is_none());
    send(&mut chat, [Message::Copy(0)]);
    assert_eq!(chat.copied(), Some(0));
    assert!(chat.context_menu().is_none());
    assert_eq!(chat.items()[0].copy_text(), "oi");
}

#[test]
fn image_attachments_are_validated_and_sent() {
    assert_eq!(media_type_of("foto.JPG"), Some("image/jpeg"));
    assert!(media_type_of("notas.txt").is_none());
    assert!(Attachment::from_bytes("a.txt".into(), PNG.to_vec()).is_err());
    assert!(Attachment::from_bytes("a.png".into(), vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    let attachment = Attachment::from_bytes("a.png".into(), PNG.to_vec()).unwrap();
    let mut chat = chat();
    assert!(!chat.can_send());
    send(
        &mut chat,
        [Message::AttachmentLoaded(Ok(attachment.clone()))],
    );
    assert!(chat.can_send());
    send(
        &mut chat,
        [
            Message::AttachmentLoaded(Ok(attachment)),
            Message::RemoveAttachment(1),
            Message::AttachmentLoaded(Err("grande".into())),
        ],
    );
    assert_eq!(chat.attachments().len(), 1);
    assert!(matches!(&chat.items()[0], Item::Error(e) if e == "grande"));
    send(&mut chat, [Message::Send]);
    assert!(chat.attachments().is_empty());
    assert!(
        matches!(&chat.items()[1], Item::User { text, attachments } if text.is_empty() && attachments.len() == 1)
    );
    let _ = chat.view();
}

#[tokio::test]
async fn idle_stream_ends_with_an_error() {
    let pending = futures::stream::pending::<AiEvent>().boxed();
    let events: Vec<AiEvent> = with_idle_timeout(pending, std::time::Duration::from_millis(20))
        .collect()
        .await;
    assert_eq!(events, vec![AiEvent::Error(IDLE_ANSWER.to_string())]);
}
