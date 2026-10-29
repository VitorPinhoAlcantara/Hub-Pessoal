use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{self, UnboundedSender};
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use hub_core::HubContext;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use crate::provider::{
    AiEvent, AiProvider, AiRequest, AttachmentInput, McpEndpoint, SYSTEM_PROMPT,
};

const MAX_ROUNDS: usize = 12;
const MAX_ERROR_BODY: usize = 500;
const TOOL_PREFIX: &str = "mcp__hub__";
const SYSTEM_SUFFIX: &str =
    " Neste provedor as ferramentas do hub aparecem sem o prefixo mcp__hub.";

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

type Sessions = Arc<Mutex<HashMap<String, Vec<Value>>>>;
type McpClient = RunningService<rmcp::RoleClient, ()>;

pub struct OpenAiCompatible {
    pub ctx: HubContext,
    pub mcp: Option<McpEndpoint>,
    sessions: Sessions,
}

impl OpenAiCompatible {
    pub fn new(ctx: HubContext, mcp: Option<McpEndpoint>) -> Self {
        Self {
            ctx,
            mcp,
            sessions: Sessions::default(),
        }
    }
}

#[derive(Debug, Default)]
struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Value> {
        self.buffer.extend_from_slice(bytes);
        let mut chunks = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data) {
                chunks.push(value);
            }
        }
        chunks
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct TurnAccumulator {
    text: String,
    calls: BTreeMap<usize, PendingCall>,
}

impl TurnAccumulator {
    fn apply(&mut self, chunk: &Value) -> Option<String> {
        let delta = &chunk["choices"][0]["delta"];
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            let pending = self.calls.entry(index).or_default();
            if let Some(id) = call["id"].as_str() {
                pending.id = id.to_string();
            }
            if let Some(name) = call["function"]["name"].as_str() {
                pending.name.push_str(name);
            }
            if let Some(arguments) = call["function"]["arguments"].as_str() {
                pending.arguments.push_str(arguments);
            }
        }
        let content = delta["content"].as_str().filter(|text| !text.is_empty())?;
        self.text.push_str(content);
        Some(content.to_string())
    }

    fn finish(self, round: usize) -> (String, Vec<PendingCall>) {
        let calls = self
            .calls
            .into_values()
            .enumerate()
            .map(|(position, mut call)| {
                if call.id.is_empty() {
                    call.id = format!("call_{round}_{position}");
                }
                call
            })
            .collect();
        (self.text, calls)
    }
}

fn attachment_part(attachment: &AttachmentInput) -> Value {
    match attachment {
        AttachmentInput::Image { media_type, data } => json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{media_type};base64,{data}") }
        }),
        AttachmentInput::Text { name, content } => json!({
            "type": "text",
            "text": format!("Arquivo anexado: {name}\n\n{content}")
        }),
        AttachmentInput::Pdf { .. } => json!({
            "type": "text",
            "text": "Um PDF foi anexado, mas PDFs não são suportados por este provedor."
        }),
    }
}

fn user_message(request: &AiRequest) -> Value {
    if request.attachments.is_empty() {
        return json!({ "role": "user", "content": request.prompt });
    }
    let mut parts: Vec<Value> = request.attachments.iter().map(attachment_part).collect();
    if !request.prompt.is_empty() {
        parts.push(json!({ "type": "text", "text": request.prompt }));
    }
    json!({ "role": "user", "content": parts })
}

fn tool_to_openai(tool: &Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool["name"],
            "description": tool.get("description").cloned().unwrap_or_else(|| json!("")),
            "parameters": tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
        }
    })
}

fn request_body(model: &str, messages: &[Value], tools: &[Value]) -> Value {
    let mut body = json!({ "model": model, "messages": messages, "stream": true });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    body
}

fn assistant_message(text: &str, calls: &[PendingCall]) -> Value {
    if calls.is_empty() {
        return json!({ "role": "assistant", "content": text });
    }
    let tool_calls: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "type": "function",
                "function": { "name": call.name, "arguments": call.arguments }
            })
        })
        .collect();
    json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { json!(text) },
        "tool_calls": tool_calls
    })
}

fn is_local(url: &str) -> bool {
    ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|host| url.contains(host))
}

fn connection_message(url: &str) -> String {
    let mut message = format!("Não foi possível conectar em {url}.");
    if is_local(url) {
        message.push_str(" Confira se o Ollama ou o LM Studio está em execução.");
    } else {
        message.push_str(" Confira a URL e a conexão com a internet.");
    }
    message
}

fn truncate(text: &str) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(MAX_ERROR_BODY) {
        Some((end, _)) => format!("{}...", &trimmed[..end]),
        None => trimmed.to_string(),
    }
}

fn new_session_id() -> String {
    let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    format!("api_{counter}_{millis}")
}

fn tool_result_text(result: &rmcp::model::CallToolResult) -> String {
    serde_json::to_value(&result.content)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn connect_mcp(endpoint: &McpEndpoint) -> Option<(McpClient, Vec<Value>)> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(endpoint.url.clone())
            .auth_header(endpoint.token.clone()),
    );
    let client = match ().serve(transport).await {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!("falha ao conectar no servidor MCP do hub: {error}");
            return None;
        }
    };
    match client.list_all_tools().await {
        Ok(tools) => {
            let tools = tools
                .iter()
                .filter_map(|tool| serde_json::to_value(tool).ok())
                .map(|tool| tool_to_openai(&tool))
                .collect();
            Some((client, tools))
        }
        Err(error) => {
            tracing::warn!("falha ao listar ferramentas do hub: {error}");
            None
        }
    }
}

async fn call_tool(client: &McpClient, call: &PendingCall) -> (String, bool) {
    let arguments = if call.arguments.trim().is_empty() {
        Ok(serde_json::Map::new())
    } else {
        serde_json::from_str::<Value>(&call.arguments)
            .map_err(|error| error.to_string())
            .and_then(|value| match value {
                Value::Object(map) => Ok(map),
                _ => Err("os argumentos não são um objeto JSON".to_string()),
            })
    };
    let arguments = match arguments {
        Ok(arguments) => arguments,
        Err(error) => return (format!("Argumentos inválidos: {error}"), true),
    };
    let params = CallToolRequestParams::new(call.name.clone()).with_arguments(arguments);
    match client.call_tool(params).await {
        Ok(result) => (tool_result_text(&result), result.is_error.unwrap_or(false)),
        Err(error) => (format!("Falha ao chamar a ferramenta: {error}"), true),
    }
}

struct ApiSettings {
    endpoint: String,
    model: String,
    key: String,
}

async fn stream_turn(
    http: &reqwest::Client,
    settings: &ApiSettings,
    body: &Value,
    round: usize,
    tx: &UnboundedSender<AiEvent>,
) -> Result<(String, Vec<PendingCall>), String> {
    let mut request = http.post(&settings.endpoint).json(body);
    if !settings.key.is_empty() {
        request = request.bearer_auth(&settings.key);
    }
    let response = request.send().await.map_err(|error| {
        if error.is_connect() {
            connection_message(&settings.endpoint)
        } else {
            format!("Falha na requisição para {}: {error}", settings.endpoint)
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("A API respondeu {status}: {}", truncate(&text)));
    }
    let mut parser = SseParser::default();
    let mut turn = TurnAccumulator::default();
    let mut bytes = response.bytes_stream();
    while let Some(chunk) = bytes.next().await {
        let chunk =
            chunk.map_err(|error| format!("A conexão com a API foi interrompida: {error}"))?;
        for value in parser.feed(&chunk) {
            if let Some(delta) = turn.apply(&value) {
                let _ = tx.unbounded_send(AiEvent::TextDelta(delta));
            }
        }
    }
    Ok(turn.finish(round))
}

async fn run(
    settings: ApiSettings,
    mcp: Option<McpEndpoint>,
    sessions: Sessions,
    request: AiRequest,
    tx: UnboundedSender<AiEvent>,
) {
    let (id, mut messages) = match &request.session {
        Some(id) => {
            let history = sessions.lock().ok().and_then(|map| map.get(id).cloned());
            (
                id.clone(),
                history.unwrap_or_else(|| vec![system_message()]),
            )
        }
        None => {
            let id = new_session_id();
            let _ = tx.unbounded_send(AiEvent::Session(id.clone()));
            (id, vec![system_message()])
        }
    };
    messages.push(user_message(&request));

    let connection = match &mcp {
        Some(endpoint) => connect_mcp(endpoint).await,
        None => None,
    };
    let tools: Vec<Value> = connection
        .as_ref()
        .map(|(_, tools)| tools.clone())
        .unwrap_or_default();

    let http = reqwest::Client::new();
    for round in 0..MAX_ROUNDS {
        let body = request_body(&settings.model, &messages, &tools);
        let (text, calls) = match stream_turn(&http, &settings, &body, round, &tx).await {
            Ok(turn) => turn,
            Err(message) => {
                let _ = tx.unbounded_send(AiEvent::Error(message));
                return;
            }
        };
        messages.push(assistant_message(&text, &calls));
        let Some((client, _)) = connection.as_ref().filter(|_| !calls.is_empty()) else {
            if let Ok(mut map) = sessions.lock() {
                map.insert(id.clone(), messages);
            }
            let _ = tx.unbounded_send(AiEvent::Done {
                session: Some(id),
                is_error: false,
                summary: None,
            });
            return;
        };
        for call in &calls {
            let _ = tx.unbounded_send(AiEvent::ToolUse {
                id: call.id.clone(),
                name: format!("{TOOL_PREFIX}{}", call.name),
                input: call.arguments.clone(),
            });
            let (output, is_error) = call_tool(client, call).await;
            let _ = tx.unbounded_send(AiEvent::ToolResult {
                id: call.id.clone(),
                output: output.clone(),
                is_error,
            });
            messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": output }));
        }
    }
    let _ = tx.unbounded_send(AiEvent::Error(format!(
        "A IA fez chamadas de ferramentas demais ({MAX_ROUNDS} rodadas) e foi interrompida."
    )));
}

fn system_message() -> Value {
    json!({ "role": "system", "content": format!("{SYSTEM_PROMPT}{SYSTEM_SUFFIX}") })
}

impl AiProvider for OpenAiCompatible {
    fn label(&self) -> &'static str {
        "API ou modelo local"
    }

    fn send(&self, request: AiRequest) -> BoxStream<'static, AiEvent> {
        let ai = self.ctx.config().ai;
        let model = ai.api_model.trim().to_string();
        if model.is_empty() {
            return futures::stream::once(async {
                AiEvent::Error("Escolha o modelo da API em Configurações, aba Geral.".into())
            })
            .boxed();
        }
        let settings = ApiSettings {
            endpoint: format!(
                "{}/chat/completions",
                ai.api_url.trim().trim_end_matches('/')
            ),
            model,
            key: ai.api_key.trim().to_string(),
        };
        let (tx, rx) = mpsc::unbounded();
        let driver = run(
            settings,
            self.mcp.clone(),
            self.sessions.clone(),
            request,
            tx,
        )
        .into_stream()
        .filter_map(|_| async { None });
        futures::stream::select(driver, rx).boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hub_core::clock::FakeClock;
    use hub_core::platform::FakePlatform;
    use hub_core::{Config, Database, HubPaths};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    fn chunk(delta: Value) -> String {
        format!("data: {}\n\n", json!({ "choices": [{ "delta": delta }] }))
    }

    #[test]
    fn sse_parser_handles_split_lines_and_done() {
        let mut parser = SseParser::default();
        let full = format!(
            "{}{}data: [DONE]\n\n",
            chunk(json!({"content":"Olá"})),
            chunk(json!({"content":" é"}))
        );
        let bytes = full.as_bytes();
        let cut = bytes.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let mut values = parser.feed(&bytes[..cut]);
        values.extend(parser.feed(&bytes[cut..]));
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["choices"][0]["delta"]["content"], "Olá");
    }

    #[test]
    fn accumulator_joins_text_and_tool_call_fragments() {
        let mut turn = TurnAccumulator::default();
        let first = json!({"choices":[{"delta":{"content":"Vou "}}]});
        assert_eq!(turn.apply(&first), Some("Vou ".into()));
        turn.apply(
            &json!({"choices":[{"delta":{"content":"criar","tool_calls":[
                {"index":0,"id":"c1","function":{"name":"lembrete_criar","arguments":"{\"quan"}},
                {"index":1,"function":{"name":"agenda_listar","arguments":""}}
            ]}}]}),
        );
        assert_eq!(
            turn.apply(&json!({"choices":[{"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"do\":\"x\"}"}}
            ]}}]})),
            None
        );
        let (text, calls) = turn.finish(3);
        assert_eq!(text, "Vou criar");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[0].arguments, r#"{"quando":"x"}"#);
        assert_eq!(calls[1].id, "call_3_1");
        assert_eq!(calls[1].name, "agenda_listar");
    }

    #[test]
    fn converts_mcp_tool_to_openai_format() {
        let tool = json!({
            "name": "lembrete_criar",
            "description": "Cria lembrete",
            "inputSchema": {"type":"object","properties":{"quando":{"type":"string"}}}
        });
        let converted = tool_to_openai(&tool);
        assert_eq!(converted["type"], "function");
        assert_eq!(converted["function"]["name"], "lembrete_criar");
        assert_eq!(converted["function"]["description"], "Cria lembrete");
        assert_eq!(
            converted["function"]["parameters"]["properties"]["quando"]["type"],
            "string"
        );
    }

    #[test]
    fn builds_user_message_with_attachments() {
        let request = AiRequest {
            prompt: "veja".into(),
            attachments: vec![
                AttachmentInput::Image {
                    media_type: "image/png".into(),
                    data: "AAAA".into(),
                },
                AttachmentInput::Text {
                    name: "a.txt".into(),
                    content: "oi".into(),
                },
                AttachmentInput::Pdf {
                    data: "BBBB".into(),
                },
            ],
            session: None,
        };
        let message = user_message(&request);
        let parts = message["content"].as_array().unwrap();
        assert_eq!(parts[0]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(parts[1]["text"], "Arquivo anexado: a.txt\n\noi");
        assert!(parts[2]["text"].as_str().unwrap().contains("PDF"));
        assert_eq!(parts[3]["text"], "veja");
        let plain = AiRequest {
            prompt: "oi".into(),
            attachments: vec![],
            session: None,
        };
        assert_eq!(user_message(&plain)["content"], "oi");
    }

    #[test]
    fn request_body_omits_tools_when_empty() {
        let messages = vec![json!({"role":"user","content":"oi"})];
        let body = request_body("m", &messages, &[]);
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_none());
        let with_tools = request_body("m", &messages, &[json!({"type":"function"})]);
        assert_eq!(with_tools["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn assistant_message_carries_tool_calls() {
        let calls = vec![PendingCall {
            id: "c1".into(),
            name: "x".into(),
            arguments: "{}".into(),
        }];
        let message = assistant_message("", &calls);
        assert!(message["content"].is_null());
        assert_eq!(message["tool_calls"][0]["function"]["name"], "x");
    }

    #[test]
    fn connection_message_mentions_local_servers() {
        assert!(
            connection_message("http://localhost:11434/v1/chat/completions").contains("Ollama")
        );
        assert!(
            !connection_message("https://api.openai.com/v1/chat/completions").contains("Ollama")
        );
        assert!(truncate(&"a".repeat(900)).chars().count() <= MAX_ERROR_BODY + 3);
    }

    fn provider(url: &str, model: &str) -> OpenAiCompatible {
        let mut config = Config::default();
        config.ai.api_url = url.into();
        config.ai.api_model = model.into();
        let ctx = HubContext::new(
            HubPaths::from_home("teste"),
            config,
            Database::open_in_memory().unwrap(),
            Arc::new(FakePlatform::default()),
            Arc::new(FakeClock::at("2026-10-21 14:00")),
        );
        OpenAiCompatible::new(ctx, None)
    }

    fn request(prompt: &str, session: Option<String>) -> AiRequest {
        AiRequest {
            prompt: prompt.into(),
            attachments: vec![],
            session,
        }
    }

    async fn serve_once(status: &str, body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/", listener.local_addr().unwrap());
        let status = status.to_string();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0u8; 65536];
                let _ = socket.read(&mut buffer).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        url
    }

    #[tokio::test]
    async fn empty_model_reports_error() {
        let events: Vec<AiEvent> = provider("http://x", " ")
            .send(request("oi", None))
            .collect()
            .await;
        assert_eq!(
            events,
            vec![AiEvent::Error(
                "Escolha o modelo da API em Configurações, aba Geral.".into()
            )]
        );
    }

    #[tokio::test]
    async fn streams_text_and_keeps_history() {
        let body = format!(
            "{}{}data: [DONE]\n\n",
            chunk(json!({"content":"Olá"})),
            chunk(json!({"content":"!"}))
        );
        let url = serve_once("200 OK", body).await;
        let provider = provider(&url, "modelo");
        let events: Vec<AiEvent> = provider.send(request("oi", None)).collect().await;
        let AiEvent::Session(id) = events[0].clone() else {
            panic!("sem sessão: {events:?}")
        };
        assert_eq!(events[1], AiEvent::TextDelta("Olá".into()));
        assert_eq!(events[2], AiEvent::TextDelta("!".into()));
        assert_eq!(
            events[3],
            AiEvent::Done {
                session: Some(id.clone()),
                is_error: false,
                summary: None
            }
        );
        let history = provider.sessions.lock().unwrap().get(&id).cloned().unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[2]["content"], "Olá!");
        let again: Vec<AiEvent> = provider
            .send(request("e aí", Some(id.clone())))
            .collect()
            .await;
        assert!(matches!(again.last(), Some(AiEvent::Done { .. })));
        assert_eq!(provider.sessions.lock().unwrap().get(&id).unwrap().len(), 5);
    }

    #[tokio::test]
    async fn non_success_status_includes_body() {
        let url = serve_once("401 Unauthorized", "chave inválida".into()).await;
        let events: Vec<AiEvent> = provider(&url, "m")
            .send(request("oi", None))
            .collect()
            .await;
        let AiEvent::Error(message) = events.last().unwrap() else {
            panic!("{events:?}")
        };
        assert!(message.contains("401") && message.contains("chave inválida"));
    }
}
