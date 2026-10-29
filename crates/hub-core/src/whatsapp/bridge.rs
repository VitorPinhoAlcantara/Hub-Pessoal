use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::{FutureExt, Stream, StreamExt};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::protocol::{
    self, Incoming, Request, WireChatInfo, WireConversation, WireMessage, WireParticipant,
};
use crate::platform::{self, CommandSpec};

pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);

type Reply = oneshot::Sender<Result<Value, String>>;

struct Outgoing {
    request: Request,
    reply: Option<Reply>,
}

#[derive(Clone)]
pub struct BridgeHandle {
    tx: mpsc::UnboundedSender<Outgoing>,
}

impl std::fmt::Debug for BridgeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BridgeHandle")
    }
}

impl BridgeHandle {
    pub fn send(&self, request: Request) -> bool {
        self.tx
            .unbounded_send(Outgoing {
                request,
                reply: None,
            })
            .is_ok()
    }

    pub async fn call(&self, request: Request) -> Result<Value, String> {
        self.call_within(request, CALL_TIMEOUT).await
    }

    async fn call_within(&self, request: Request, limit: Duration) -> Result<Value, String> {
        let (reply, answer) = oneshot::channel();
        self.tx
            .unbounded_send(Outgoing {
                request,
                reply: Some(reply),
            })
            .map_err(|_| "o WhatsApp não está aberto".to_string())?;
        match tokio::time::timeout(limit, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("o WhatsApp foi encerrado".to_string()),
            Err(_) => Err("o WhatsApp não respondeu a tempo".to_string()),
        }
    }

    pub async fn conversations(&self) -> Result<Vec<WireConversation>, String> {
        let value = self.call(Request::ListConversations).await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub async fn chat_info(&self, jids: Vec<String>) -> Result<Vec<WireChatInfo>, String> {
        let value = self.call(Request::ChatInfo { jids }).await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub async fn download_media(&self, id: &str, media: &str) -> Result<PathBuf, String> {
        let value = self
            .call(Request::DownloadMedia {
                id: id.to_string(),
                media: media.to_string(),
            })
            .await?;
        value
            .get("caminho")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| "o WhatsApp não informou onde salvou a imagem".to_string())
    }

    pub async fn send_text(&self, jid: &str, text: &str) -> Result<WireMessage, String> {
        self.send_with_mentions(jid, text, Vec::new()).await
    }

    pub async fn send_with_mentions(
        &self,
        jid: &str,
        text: &str,
        mentions: Vec<String>,
    ) -> Result<WireMessage, String> {
        let value = self
            .call(Request::Send {
                jid: jid.to_string(),
                text: text.to_string(),
                mentions,
            })
            .await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub async fn participants(&self, jid: &str) -> Result<Vec<WireParticipant>, String> {
        let value = self
            .call(Request::Participants {
                jid: jid.to_string(),
            })
            .await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    pub async fn send_file(
        &self,
        jid: &str,
        path: &std::path::Path,
        caption: &str,
    ) -> Result<WireMessage, String> {
        let value = self
            .call_within(
                Request::SendFile {
                    jid: jid.to_string(),
                    path: path.to_string_lossy().into_owned(),
                    caption: caption.to_string(),
                },
                UPLOAD_TIMEOUT,
            )
            .await?;
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone)]
pub enum BridgeEvent {
    Ready(BridgeHandle),
    Qr(String),
    Connected(String),
    Message(WireMessage),
    Disconnected(String),
    Archived(String, bool),
    Error(String),
    Exited(String),
}

pub fn spawn(
    command: CommandSpec,
    cwd: PathBuf,
) -> impl Stream<Item = BridgeEvent> + Send + 'static {
    let (out, events) = mpsc::unbounded();
    let driver = drive(command, cwd, out)
        .into_stream()
        .filter_map(|_| async { None });
    futures::stream::select(driver, events)
}

async fn drive(command: CommandSpec, cwd: PathBuf, out: mpsc::UnboundedSender<BridgeEvent>) {
    let mut process = tokio::process::Command::new(&command.program);
    process
        .args(&command.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    platform::hide_console(&mut process);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = out.unbounded_send(BridgeEvent::Exited(format!(
                "Não foi possível iniciar o {} ({error}). Compile o sidecar e coloque ao lado do hub.",
                command.program.to_string_lossy()
            )));
            return;
        }
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = out.unbounded_send(BridgeEvent::Exited("sem acesso ao sidecar".to_string()));
        return;
    };
    let (tx, mut outgoing) = mpsc::unbounded::<Outgoing>();
    let _ = out.unbounded_send(BridgeEvent::Ready(BridgeHandle { tx }));
    let mut lines = BufReader::new(stdout).lines();
    let mut pending: HashMap<u64, Reply> = HashMap::new();
    let mut next_id = 0u64;
    let mut outgoing_open = true;
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => match protocol::parse(&line) {
                    Some(Incoming::Response { id, result }) => match pending.remove(&id) {
                        Some(reply) => {
                            let _ = reply.send(result);
                        }
                        None => {
                            if let Err(error) = result {
                                let _ = out.unbounded_send(BridgeEvent::Error(error));
                            }
                        }
                    },
                    Some(Incoming::Qr(code)) => {
                        let _ = out.unbounded_send(BridgeEvent::Qr(code));
                    }
                    Some(Incoming::Connected(jid)) => {
                        let _ = out.unbounded_send(BridgeEvent::Connected(jid));
                    }
                    Some(Incoming::Message(message)) => {
                        let _ = out.unbounded_send(BridgeEvent::Message(message));
                    }
                    Some(Incoming::Disconnected(reason)) => {
                        let _ = out.unbounded_send(BridgeEvent::Disconnected(reason));
                    }
                    Some(Incoming::Archived { jid, archived }) => {
                        let _ = out.unbounded_send(BridgeEvent::Archived(jid, archived));
                    }
                    Some(Incoming::Error(error)) => {
                        let _ = out.unbounded_send(BridgeEvent::Error(error));
                    }
                    None => {}
                },
                _ => break,
            },
            message = outgoing.next(), if outgoing_open => match message {
                Some(Outgoing { request, reply }) => {
                    next_id += 1;
                    if let Some(reply) = reply {
                        pending.insert(next_id, reply);
                    }
                    if stdin.write_all(protocol::encode(&request, next_id).as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
                None => {
                    outgoing_open = false;
                    let _ = stdin
                        .write_all(protocol::encode(&Request::Shutdown, next_id + 1).as_bytes())
                        .await;
                }
            },
        }
    }
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err("o WhatsApp foi encerrado".to_string()));
    }
    let status = child.wait().await;
    let reason = match status {
        Ok(status) if status.success() => "WhatsApp encerrado".to_string(),
        Ok(status) => format!("O sidecar do WhatsApp terminou ({status})"),
        Err(error) => error.to_string(),
    };
    let _ = out.unbounded_send(BridgeEvent::Exited(reason));
}
