use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;

use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use hub_core::platform::{self, CommandSpec};
use serde_json::Value;

use crate::provider::{
    AiEvent, AiProvider, AiRequest, McpEndpoint, SYSTEM_PROMPT, drive_cli, stage_attachments,
};

const TOKEN_ENV: &str = "HUB_MCP_TOKEN";
const INSTALL: &str = "Instale com npm install -g @openai/codex e faça login com codex login.";

#[derive(Debug, Default)]
pub struct CodexParser {
    thread: Option<String>,
    last_text: Option<String>,
    started: HashSet<String>,
}

fn tool_use(item: &Value) -> AiEvent {
    AiEvent::ToolUse {
        id: item["id"].as_str().unwrap_or_default().to_string(),
        name: format!(
            "mcp__{}__{}",
            item["server"].as_str().unwrap_or("hub"),
            item["tool"].as_str().unwrap_or_default()
        ),
        input: item
            .get("arguments")
            .map(Value::to_string)
            .unwrap_or_default(),
    }
}

fn tool_output(item: &Value) -> String {
    if let Some(message) = item["error"]["message"].as_str() {
        return message.to_string();
    }
    let content = item["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if content.is_empty() {
        item["result"]
            .get("structured_content")
            .filter(|value| !value.is_null())
            .map(Value::to_string)
            .unwrap_or_default()
    } else {
        content
    }
}

impl CodexParser {
    pub fn parse(&mut self, line: &str) -> Vec<AiEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "thread.started" => match value.get("thread_id").and_then(Value::as_str) {
                Some(id) => {
                    self.thread = Some(id.to_string());
                    vec![AiEvent::Session(id.to_string())]
                }
                None => Vec::new(),
            },
            "item.started" => {
                let item = &value["item"];
                if item["type"].as_str() == Some("mcp_tool_call") {
                    self.started
                        .insert(item["id"].as_str().unwrap_or_default().to_string());
                    vec![tool_use(item)]
                } else {
                    Vec::new()
                }
            }
            "item.completed" => {
                let item = &value["item"];
                match item["type"].as_str() {
                    Some("agent_message") => {
                        let text = item["text"].as_str().unwrap_or_default();
                        if text.is_empty() {
                            return Vec::new();
                        }
                        self.last_text = Some(text.to_string());
                        vec![AiEvent::Text(text.to_string())]
                    }
                    Some("mcp_tool_call") => {
                        let id = item["id"].as_str().unwrap_or_default().to_string();
                        let mut events = Vec::new();
                        if !self.started.contains(&id) {
                            events.push(tool_use(item));
                        }
                        events.push(AiEvent::ToolResult {
                            id,
                            output: tool_output(item),
                            is_error: item["status"].as_str() == Some("failed")
                                || item.get("error").is_some_and(|e| !e.is_null()),
                        });
                        events
                    }
                    _ => Vec::new(),
                }
            }
            "turn.completed" => vec![AiEvent::Done {
                session: self.thread.clone(),
                is_error: false,
                summary: self.last_text.clone(),
            }],
            "turn.failed" => vec![AiEvent::Done {
                session: self.thread.clone(),
                is_error: true,
                summary: value["error"]["message"].as_str().map(str::to_string),
            }],
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodexCli {
    pub command: CommandSpec,
    pub cwd: PathBuf,
    pub mcp: Option<McpEndpoint>,
}

impl CodexCli {
    pub fn new(cwd: PathBuf, mcp: Option<McpEndpoint>) -> Self {
        Self {
            command: platform::npm_cli("codex"),
            cwd,
            mcp,
        }
    }

    pub fn args(&self, session: Option<&str>, images: &[PathBuf]) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["exec".into()];
        if let Some(session) = session {
            args.push("resume".into());
            args.push(session.into());
        }
        for image in images {
            args.push("-i".into());
            args.push(image.clone().into_os_string());
        }
        let mut flags = vec![
            "--json".to_string(),
            "--skip-git-repo-check".to_string(),
            "--ignore-user-config".to_string(),
            "-c".to_string(),
            "sandbox_mode=read-only".to_string(),
            "-c".to_string(),
            "approval_policy=never".to_string(),
            "-c".to_string(),
            "features.shell_tool=false".to_string(),
        ];
        if let Some(mcp) = &self.mcp {
            for setting in [
                format!("mcp_servers.hub.url={}", mcp.url),
                format!("mcp_servers.hub.bearer_token_env_var={TOKEN_ENV}"),
                "mcp_servers.hub.default_tools_approval_mode=approve".to_string(),
            ] {
                flags.push("-c".into());
                flags.push(setting);
            }
        }
        args.extend(flags.into_iter().map(OsString::from));
        args.push("-".into());
        args
    }
}

impl AiProvider for CodexCli {
    fn label(&self) -> &'static str {
        "Codex CLI"
    }

    fn send(&self, request: AiRequest) -> BoxStream<'static, AiEvent> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let cli = self.clone();
        let driver = async move {
            let folder = cli.cwd.join("codex");
            let (images, inline) =
                stage_attachments(&folder.join("anexos"), &request.attachments).await;
            let mut prompt = String::new();
            if request.session.is_none() {
                prompt.push_str(SYSTEM_PROMPT);
                prompt.push_str("\n\n");
            }
            prompt.push_str(&inline);
            prompt.push_str(&request.prompt);
            let _ = tokio::fs::create_dir_all(&folder).await;
            let mut command = tokio::process::Command::new(&cli.command.program);
            command
                .args(&cli.command.args)
                .args(cli.args(request.session.as_deref(), &images))
                .current_dir(&folder)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            if let Some(mcp) = &cli.mcp {
                command.env(TOKEN_ENV, &mcp.token);
            }
            platform::hide_console(&mut command);
            let mut parser = CodexParser::default();
            drive_cli(command, prompt, tx, "Codex CLI", INSTALL, move |line| {
                parser.parse(line)
            })
            .await;
        };
        let driver = driver.into_stream().filter_map(|_| async { None });
        futures::stream::select(driver, rx).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_failure_and_documented_success_shapes() {
        let mut parser = CodexParser::default();
        let lines = [
            r#"{"type":"thread.started","thread_id":"01a189f1-239b-7263-bb8c-b2d7292f148c"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.started","item":{"id":"item_0","type":"mcp_tool_call","server":"hub","tool":"lembrete_criar","arguments":{"quando":"amanhã"},"status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"mcp_tool_call","server":"hub","tool":"lembrete_criar","arguments":{"quando":"amanhã"},"result":{"content":[{"type":"text","text":"criado"}]},"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"Pronto"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":1}}"#,
            r#"{"type":"error","message":"falhou"}"#,
            "texto solto",
        ];
        let events: Vec<AiEvent> = lines.iter().flat_map(|l| parser.parse(l)).collect();
        assert_eq!(
            events,
            vec![
                AiEvent::Session("01a189f1-239b-7263-bb8c-b2d7292f148c".into()),
                AiEvent::ToolUse {
                    id: "item_0".into(),
                    name: "mcp__hub__lembrete_criar".into(),
                    input: r#"{"quando":"amanhã"}"#.into(),
                },
                AiEvent::ToolResult {
                    id: "item_0".into(),
                    output: "criado".into(),
                    is_error: false,
                },
                AiEvent::Text("Pronto".into()),
                AiEvent::Done {
                    session: Some("01a189f1-239b-7263-bb8c-b2d7292f148c".into()),
                    is_error: false,
                    summary: Some("Pronto".into()),
                },
            ]
        );
        let mut parser = CodexParser::default();
        let failed = r#"{"type":"turn.failed","error":{"message":"Your access token could not be refreshed."}}"#;
        assert_eq!(
            parser.parse(failed),
            vec![AiEvent::Done {
                session: None,
                is_error: true,
                summary: Some("Your access token could not be refreshed.".into()),
            }]
        );
    }

    #[test]
    fn args_cover_mcp_resume_and_images() {
        let mcp = McpEndpoint {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "t".into(),
        };
        let cli = CodexCli::new(PathBuf::from("."), Some(mcp));
        let args: Vec<String> = cli
            .args(Some("abc"), &[PathBuf::from("a.png")])
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..3], ["exec", "resume", "abc"]);
        assert!(args.contains(&"mcp_servers.hub.url=http://127.0.0.1:1/mcp".to_string()));
        assert!(args.windows(2).any(|w| w == ["-i", "a.png"]));
        assert!(args.contains(&"sandbox_mode=read-only".to_string()));
        assert_eq!(args.last().unwrap(), "-");
        let plain: Vec<String> = CodexCli::new(PathBuf::from("."), None)
            .args(None, &[])
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!plain.iter().any(|a| a.starts_with("mcp_servers")));
        assert!(!plain.contains(&"resume".to_string()));
    }
}
