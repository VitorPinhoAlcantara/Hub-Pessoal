use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use hub_core::platform::{self, CommandSpec};
use serde_json::{Value, json};

use crate::provider::{
    AiEvent, AiProvider, AiRequest, McpEndpoint, SYSTEM_PROMPT, drive_cli, stage_attachments,
};

const TOKEN_ENV: &str = "HUB_MCP_TOKEN";
const INSTALL: &str = "Instale com npm install -g @google/gemini-cli e faça login abrindo o gemini uma vez no terminal.";
const BUILTIN_TOOLS: [&str; 13] = [
    "run_shell_command",
    "write_file",
    "replace",
    "read_file",
    "read_many_files",
    "list_directory",
    "glob",
    "search_file_content",
    "grep_search",
    "web_fetch",
    "google_web_search",
    "save_memory",
    "write_todos",
];

#[derive(Debug, Default)]
pub struct GeminiParser {
    session: Option<String>,
    text: String,
    names: HashMap<String, String>,
}

fn hub_tool_name(name: &str) -> String {
    match name.strip_prefix("mcp_hub_") {
        Some(rest) => format!("mcp__hub__{rest}"),
        None => name.to_string(),
    }
}

impl GeminiParser {
    pub fn parse(&mut self, line: &str) -> Vec<AiEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "init" => match value.get("session_id").and_then(Value::as_str) {
                Some(id) => {
                    self.session = Some(id.to_string());
                    vec![AiEvent::Session(id.to_string())]
                }
                None => Vec::new(),
            },
            "message" if value["role"].as_str() == Some("assistant") => {
                let content = value["content"].as_str().unwrap_or_default();
                if content.is_empty() {
                    return Vec::new();
                }
                self.text.push_str(content);
                if value["delta"].as_bool().unwrap_or(false) {
                    vec![AiEvent::TextDelta(content.to_string())]
                } else {
                    vec![AiEvent::Text(content.to_string())]
                }
            }
            "tool_use" => {
                let id = value["tool_id"].as_str().unwrap_or_default().to_string();
                let name = hub_tool_name(value["tool_name"].as_str().unwrap_or_default());
                self.names.insert(id.clone(), name.clone());
                vec![AiEvent::ToolUse {
                    id,
                    name,
                    input: value
                        .get("parameters")
                        .map(Value::to_string)
                        .unwrap_or_default(),
                }]
            }
            "tool_result" => {
                let is_error = value["status"].as_str() == Some("error");
                let output = match (value["output"].as_str(), value["error"]["message"].as_str()) {
                    (Some(output), _) if !output.is_empty() => output,
                    (_, Some(message)) => message,
                    _ => "",
                };
                vec![AiEvent::ToolResult {
                    id: value["tool_id"].as_str().unwrap_or_default().to_string(),
                    output: output.to_string(),
                    is_error,
                }]
            }
            "error" if value["severity"].as_str() == Some("error") => {
                vec![AiEvent::Error(
                    value["message"].as_str().unwrap_or_default().to_string(),
                )]
            }
            "result" => {
                let is_error = value["status"].as_str() == Some("error");
                let summary = if is_error {
                    value["error"]["message"].as_str().map(str::to_string)
                } else if self.text.is_empty() {
                    None
                } else {
                    Some(self.text.clone())
                };
                vec![AiEvent::Done {
                    session: self.session.clone(),
                    is_error,
                    summary,
                }]
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeminiCli {
    pub command: CommandSpec,
    pub cwd: PathBuf,
    pub mcp: Option<McpEndpoint>,
}

impl GeminiCli {
    pub fn new(cwd: PathBuf, mcp: Option<McpEndpoint>) -> Self {
        Self {
            command: platform::npm_cli("gemini"),
            cwd,
            mcp,
        }
    }

    pub fn settings(&self) -> Value {
        let mut settings = json!({
            "tools": { "exclude": BUILTIN_TOOLS },
            "general": { "defaultApprovalMode": "yolo" }
        });
        if let Some(mcp) = &self.mcp {
            settings["mcpServers"] = json!({
                "hub": {
                    "httpUrl": mcp.url,
                    "headers": { "Authorization": format!("Bearer ${TOKEN_ENV}") },
                    "trust": true
                }
            });
            settings["mcp"] = json!({ "allowed": ["hub"] });
        }
        settings
    }

    pub fn args(&self, session: Option<&str>) -> Vec<OsString> {
        let mut args: Vec<String> = ["--output-format", "stream-json", "--approval-mode", "yolo"]
            .into_iter()
            .map(String::from)
            .collect();
        if self.mcp.is_some() {
            args.push("--allowed-mcp-server-names".into());
            args.push("hub".into());
        }
        if let Some(session) = session {
            args.push("--resume".into());
            args.push(session.into());
        }
        args.into_iter().map(OsString::from).collect()
    }
}

async fn prepare(folder: &Path, cli: &GeminiCli) {
    let _ = tokio::fs::create_dir_all(folder.join(".gemini")).await;
    let _ = tokio::fs::write(folder.join("GEMINI.md"), SYSTEM_PROMPT).await;
    let _ = tokio::fs::write(
        folder.join(".gemini").join("settings.json"),
        cli.settings().to_string(),
    )
    .await;
}

impl AiProvider for GeminiCli {
    fn label(&self) -> &'static str {
        "Gemini CLI"
    }

    fn send(&self, request: AiRequest) -> BoxStream<'static, AiEvent> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let cli = self.clone();
        let driver = async move {
            let folder = cli.cwd.join("gemini");
            prepare(&folder, &cli).await;
            let (images, inline) =
                stage_attachments(&folder.join("anexos"), &request.attachments).await;
            let mut prompt = String::new();
            for image in &images {
                if let Some(name) = image.file_name().and_then(|name| name.to_str()) {
                    prompt.push_str(&format!("@anexos/{name} "));
                }
            }
            prompt.push_str(&inline);
            prompt.push_str(&request.prompt);
            let mut command = tokio::process::Command::new(&cli.command.program);
            command
                .args(&cli.command.args)
                .args(cli.args(request.session.as_deref()))
                .current_dir(&folder)
                .env("GEMINI_CLI_TRUST_WORKSPACE", "true")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            if let Some(mcp) = &cli.mcp {
                command.env(TOKEN_ENV, &mcp.token);
            }
            platform::hide_console(&mut command);
            let mut parser = GeminiParser::default();
            drive_cli(command, prompt, tx, "Gemini CLI", INSTALL, move |line| {
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
    fn parses_documented_stream_json_events() {
        let mut parser = GeminiParser::default();
        let lines = [
            r#"{"type":"init","timestamp":"2025-10-10T12:00:00.000Z","session_id":"s1","model":"gemini-2.5-pro"}"#,
            r#"{"type":"message","timestamp":"t","role":"user","content":"oi"}"#,
            r#"{"type":"message","timestamp":"t","role":"assistant","content":"Cri","delta":true}"#,
            r#"{"type":"message","timestamp":"t","role":"assistant","content":"ando","delta":true}"#,
            r#"{"type":"tool_use","timestamp":"t","tool_name":"mcp_hub_lembrete_criar","tool_id":"t1","parameters":{"quando":"amanhã"}}"#,
            r#"{"type":"tool_result","timestamp":"t","tool_id":"t1","status":"success","output":"criado"}"#,
            r#"{"type":"tool_result","timestamp":"t","tool_id":"t2","status":"error","error":{"type":"x","message":"falhou"}}"#,
            r#"{"type":"error","timestamp":"t","severity":"warning","message":"aviso"}"#,
            r#"{"type":"result","timestamp":"t","status":"success","stats":{"total_tokens":10}}"#,
            "lixo",
        ];
        let events: Vec<AiEvent> = lines.iter().flat_map(|l| parser.parse(l)).collect();
        assert_eq!(
            events,
            vec![
                AiEvent::Session("s1".into()),
                AiEvent::TextDelta("Cri".into()),
                AiEvent::TextDelta("ando".into()),
                AiEvent::ToolUse {
                    id: "t1".into(),
                    name: "mcp__hub__lembrete_criar".into(),
                    input: r#"{"quando":"amanhã"}"#.into(),
                },
                AiEvent::ToolResult {
                    id: "t1".into(),
                    output: "criado".into(),
                    is_error: false,
                },
                AiEvent::ToolResult {
                    id: "t2".into(),
                    output: "falhou".into(),
                    is_error: true,
                },
                AiEvent::Done {
                    session: Some("s1".into()),
                    is_error: false,
                    summary: Some("Criando".into()),
                },
            ]
        );
    }

    #[test]
    fn failed_result_is_an_error_done() {
        let mut parser = GeminiParser::default();
        let line =
            r#"{"type":"result","status":"error","error":{"type":"auth","message":"sem login"}}"#;
        assert_eq!(
            parser.parse(line),
            vec![AiEvent::Done {
                session: None,
                is_error: true,
                summary: Some("sem login".into()),
            }]
        );
    }

    #[test]
    fn settings_and_args_wire_the_hub() {
        let mcp = McpEndpoint {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "t".into(),
        };
        let cli = GeminiCli::new(PathBuf::from("."), Some(mcp));
        let settings = cli.settings();
        assert_eq!(
            settings["mcpServers"]["hub"]["httpUrl"],
            "http://127.0.0.1:1/mcp"
        );
        assert_eq!(
            settings["mcpServers"]["hub"]["headers"]["Authorization"],
            "Bearer $HUB_MCP_TOKEN"
        );
        assert_eq!(settings["tools"]["exclude"][0], "run_shell_command");
        let args: Vec<String> = cli
            .args(Some("s1"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w == ["--resume", "s1"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["--output-format", "stream-json"])
        );
        let plain = GeminiCli::new(PathBuf::from("."), None);
        assert!(plain.settings().get("mcpServers").is_none());
        assert!(!plain.args(None).iter().any(|a| a == "--resume"));
    }
}
