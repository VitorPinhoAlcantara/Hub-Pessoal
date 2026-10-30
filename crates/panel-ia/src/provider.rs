use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;

use futures::channel::mpsc;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use hub_core::platform::{self, CommandSpec};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub const SYSTEM_PROMPT: &str = "Você é a assistente do hub pessoal do usuário, um app de desktop. Responda em português do Brasil, de forma curta. Use as ferramentas do hub (prefixo mcp__hub) para agenda, rotinas, lembretes, scripts, atalhos, player e WhatsApp. Para tarefas no computador existe a ferramenta terminal_executar, que roda um comando no PowerShell (Windows) ou sh (Linux); cada comando pede confirmação ao usuário no hub, então prefira um comando por vez e explique o que ele faz. Datas e horas são locais. Para tarefas em lote (muitas ações seguidas ou listas grandes), prefira um único terminal_executar com um script que chama a API HTTP do hub: as variáveis HUB_API e HUB_TOKEN já existem nesse terminal, e GET $env:HUB_API com o cabeçalho Authorization: Bearer $env:HUB_TOKEN devolve a documentação com todas as ferramentas.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiEvent {
    Session(String),
    TextDelta(String),
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
    Done {
        session: Option<String>,
        is_error: bool,
        summary: Option<String>,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentInput {
    Image { media_type: String, data: String },
    Pdf { data: String },
    Text { name: String, content: String },
}

impl AttachmentInput {
    fn block(&self) -> Value {
        match self {
            AttachmentInput::Image { media_type, data } => serde_json::json!({
                "type": "image",
                "source": { "type": "base64", "media_type": media_type, "data": data }
            }),
            AttachmentInput::Pdf { data } => serde_json::json!({
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": data }
            }),
            AttachmentInput::Text { name, content } => serde_json::json!({
                "type": "text",
                "text": format!("Arquivo anexado: {name}

{content}")
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRequest {
    pub prompt: String,
    pub attachments: Vec<AttachmentInput>,
    pub session: Option<String>,
}

impl AiRequest {
    pub fn user_line(&self) -> String {
        let mut content: Vec<Value> = self
            .attachments
            .iter()
            .map(AttachmentInput::block)
            .collect();
        if !self.prompt.is_empty() {
            content.push(serde_json::json!({ "type": "text", "text": self.prompt }));
        }
        let line = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": content }
        });
        format!("{line}\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEndpoint {
    pub url: String,
    pub token: String,
}

pub trait AiProvider: Send + Sync {
    fn label(&self) -> &'static str;
    fn send(&self, request: AiRequest) -> BoxStream<'static, AiEvent>;
}

#[derive(Debug, Default)]
pub struct ClaudeParser {
    streamed_text: bool,
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl ClaudeParser {
    pub fn parse(&mut self, line: &str) -> Vec<AiEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "system" => {
                if value.get("subtype").and_then(Value::as_str) == Some("init")
                    && let Some(id) = value.get("session_id").and_then(Value::as_str)
                {
                    return vec![AiEvent::Session(id.to_string())];
                }
                Vec::new()
            }
            "stream_event" => {
                let event = &value["event"];
                match event.get("type").and_then(Value::as_str) {
                    Some("message_start") => {
                        self.streamed_text = false;
                        Vec::new()
                    }
                    Some("content_block_delta") => {
                        let delta = &event["delta"];
                        if delta.get("type").and_then(Value::as_str) == Some("text_delta") {
                            self.streamed_text = true;
                            let text = delta
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            vec![AiEvent::TextDelta(text.to_string())]
                        } else {
                            Vec::new()
                        }
                    }
                    _ => Vec::new(),
                }
            }
            "assistant" => {
                let mut events = Vec::new();
                for block in value["message"]["content"].as_array().into_iter().flatten() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") if !self.streamed_text => {
                            let text = block
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            if !text.is_empty() {
                                events.push(AiEvent::Text(text.to_string()));
                            }
                        }
                        Some("tool_use") => events.push(AiEvent::ToolUse {
                            id: block["id"].as_str().unwrap_or_default().to_string(),
                            name: block["name"].as_str().unwrap_or_default().to_string(),
                            input: block.get("input").map(Value::to_string).unwrap_or_default(),
                        }),
                        _ => {}
                    }
                }
                events
            }
            "user" => value["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
                .map(|block| AiEvent::ToolResult {
                    id: block["tool_use_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    output: text_of(&block["content"]),
                    is_error: block["is_error"].as_bool().unwrap_or(false),
                })
                .collect(),
            "result" => vec![AiEvent::Done {
                session: value
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                is_error: value["is_error"].as_bool().unwrap_or(false),
                summary: value
                    .get("result")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }],
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeCli {
    pub command: CommandSpec,
    pub cwd: PathBuf,
    pub mcp_config: Option<PathBuf>,
}

impl ClaudeCli {
    pub fn new(cwd: PathBuf, mcp_config: Option<PathBuf>) -> Self {
        Self {
            command: CommandSpec::new("claude"),
            cwd,
            mcp_config,
        }
    }

    pub fn args(&self, session: Option<&str>) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--tools",
            "",
            "--strict-mcp-config",
            "--append-system-prompt",
            SYSTEM_PROMPT,
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        if let Some(config) = &self.mcp_config {
            args.push("--mcp-config".into());
            args.push(config.clone().into_os_string());
            args.push("--allowedTools".into());
            args.push("mcp__hub".into());
        }
        if let Some(session) = session {
            args.push("--resume".into());
            args.push(session.into());
        }
        args
    }
}

impl AiProvider for ClaudeCli {
    fn label(&self) -> &'static str {
        "Claude CLI"
    }

    fn send(&self, request: AiRequest) -> BoxStream<'static, AiEvent> {
        let (tx, rx) = mpsc::unbounded();
        let mut command = tokio::process::Command::new(&self.command.program);
        command
            .args(&self.command.args)
            .args(self.args(request.session.as_deref()))
            .current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        platform::hide_console(&mut command);
        let mut parser = ClaudeParser::default();
        let driver = drive_cli(
            command,
            request.user_line(),
            tx,
            "Claude CLI",
            "Confira se o comando claude está instalado e logado.",
            move |line| parser.parse(line),
        )
            .into_stream()
            .filter_map(|_| async { None });
        futures::stream::select(driver, rx).boxed()
    }
}

pub(crate) async fn drive_cli(
    mut command: tokio::process::Command,
    input: String,
    tx: mpsc::UnboundedSender<AiEvent>,
    name: &str,
    install: &str,
    mut parse: impl FnMut(&str) -> Vec<AiEvent>,
) {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = tx.unbounded_send(AiEvent::Error(format!(
                "Não foi possível iniciar o {name} ({error}). {install}"
            )));
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        })
    });
    let mut finished = false;
    let mut any_event = false;
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            for event in parse(&line) {
                any_event = true;
                finished |= matches!(event, AiEvent::Done { .. });
                let _ = tx.unbounded_send(event);
            }
        }
    }
    let status = child.wait().await;
    let stderr = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if !finished {
        let detail = stderr.trim();
        let detail = match (status, detail.is_empty()) {
            (_, false) => detail.to_string(),
            (Ok(status), true) => format!("terminou sem resposta, {status}"),
            (Err(error), true) => error.to_string(),
        };
        let message = if any_event {
            detail
        } else {
            format!("Não foi possível iniciar o {name} ({detail}). {install}")
        };
        let _ = tx.unbounded_send(AiEvent::Error(message));
    }
}

pub(crate) async fn stage_attachments(
    dir: &std::path::Path,
    attachments: &[AttachmentInput],
) -> (Vec<PathBuf>, String) {
    use base64::Engine;
    let _ = tokio::fs::remove_dir_all(dir).await;
    let mut images = Vec::new();
    let mut inline = String::new();
    for (index, attachment) in attachments.iter().enumerate() {
        match attachment {
            AttachmentInput::Image { media_type, data } => {
                let extension = media_type.rsplit('/').next().unwrap_or("png");
                let extension = if extension == "jpeg" {
                    "jpg"
                } else {
                    extension
                };
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
                    continue;
                };
                let path = dir.join(format!("imagem{index}.{extension}"));
                if tokio::fs::create_dir_all(dir).await.is_ok()
                    && tokio::fs::write(&path, bytes).await.is_ok()
                {
                    images.push(path);
                }
            }
            AttachmentInput::Pdf { .. } => {
                inline.push_str("[Um PDF foi anexado, mas este provedor não suporta PDF.]\n\n");
            }
            AttachmentInput::Text { name, content } => {
                inline.push_str(&format!("Arquivo anexado: {name}\n\n{content}\n\n"));
            }
        }
    }
    (images, inline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_deltas_tools_and_result() {
        let mut parser = ClaudeParser::default();
        let lines = [
            r#"{"type":"system","subtype":"init","session_id":"abc"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Cri"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ando"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Criando"},{"type":"tool_use","id":"t1","name":"mcp__hub__lembrete_criar","input":{"quando":"em 1 minuto"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"criado"}],"is_error":false}]}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Pronto"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"session_id":"abc","result":"Pronto"}"#,
            "não é json",
        ];
        let events: Vec<AiEvent> = lines.iter().flat_map(|l| parser.parse(l)).collect();
        assert_eq!(
            events,
            vec![
                AiEvent::Session("abc".into()),
                AiEvent::TextDelta("Cri".into()),
                AiEvent::TextDelta("ando".into()),
                AiEvent::ToolUse {
                    id: "t1".into(),
                    name: "mcp__hub__lembrete_criar".into(),
                    input: r#"{"quando":"em 1 minuto"}"#.into(),
                },
                AiEvent::ToolResult {
                    id: "t1".into(),
                    output: "criado".into(),
                    is_error: false,
                },
                AiEvent::Text("Pronto".into()),
                AiEvent::Done {
                    session: Some("abc".into()),
                    is_error: false,
                    summary: Some("Pronto".into()),
                },
            ]
        );
    }

    #[test]
    fn user_line_carries_attachments_before_text() {
        let request = AiRequest {
            prompt: "o que é isso?".into(),
            attachments: vec![
                AttachmentInput::Image {
                    media_type: "image/png".into(),
                    data: "AAAA".into(),
                },
                AttachmentInput::Pdf {
                    data: "BBBB".into(),
                },
                AttachmentInput::Text {
                    name: "notas.txt".into(),
                    content: "linha".into(),
                },
            ],
            session: None,
        };
        let line = request.user_line();
        assert!(line.ends_with('\n'));
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["type"], "user");
        let content = value["message"]["content"].as_array().unwrap();
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["type"], "document");
        assert_eq!(content[1]["source"]["media_type"], "application/pdf");
        assert_eq!(
            content[2]["text"],
            "Arquivo anexado: notas.txt

linha"
        );
        assert_eq!(content[3]["text"], "o que é isso?");
    }

    #[test]
    fn args_include_mcp_and_resume_when_available() {
        let cli = ClaudeCli::new(PathBuf::from("."), Some(PathBuf::from("mcp.json")));
        let args: Vec<String> = cli
            .args(Some("sessao"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"stream-json".to_string()));
        assert!(args.windows(2).any(|w| w == ["--mcp-config", "mcp.json"]));
        assert!(args.windows(2).any(|w| w == ["--allowedTools", "mcp__hub"]));
        assert!(args.windows(2).any(|w| w == ["--resume", "sessao"]));
        assert!(args.windows(2).any(|w| w == ["--tools", ""]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--input-format", "stream-json"])
        );
        let plain = ClaudeCli::new(PathBuf::from("."), None).args(None);
        assert!(!plain.iter().any(|a| a == "--mcp-config"));
        assert!(!plain.iter().any(|a| a == "--resume"));
    }
}
