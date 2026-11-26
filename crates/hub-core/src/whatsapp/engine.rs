use std::collections::HashMap;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use base64::Engine as _;
use chrono::Utc;
use futures::channel::mpsc;
use futures::{Stream, StreamExt};
use rusqlite::Connection;
use tokio::sync::watch;
use whatsapp_rust::buffa::Message as _;
use whatsapp_rust::download::DownloadLimits;
use whatsapp_rust::prelude::{
    Bot, BotHandle, Client, Event, EventHandler, EventInterest, EventKind, InboundMessage, Jid,
    MessageBuilderExt, MessageExt, MessageField, wa,
};
use whatsapp_rust::store::{SqliteDatabase, SqliteDatabaseConfig};
use whatsapp_rust::types::events::{ConnectFailureReason, TempBanReason};
use whatsapp_rust::types::message::EditAttribute;
use whatsapp_rust::wacore::download::Downloadable;
use whatsapp_rust::wacore::download::MediaType;
use whatsapp_rust::wacore::store::DevicePropsOverride;
use whatsapp_rust::wacore_binary::JidExt;
use whatsapp_rust::{
    EditOptions, PresencePolicy, ProfilePictureLookup, ProfilePictureRequest, ProfilePictureTarget,
    ProfilePictureType, RevokeType, SendError, UploadOptions, schemas, waproto,
};

use super::StoredRaw;
use super::convert::{self, Content, Meta};
use super::files::{self, OutKind};
use super::protocol::{
    WireChatInfo, WireConversation, WireEdit, WireMention, WireMessage, WireParticipant,
    WireReaction, WireRevoke,
};
use super::service::SESSION_ENDED;

pub const SESSION_FILE: &str = "whatsapp-motor.db";
pub const MAIN_DB_FILE: &str = "hub.db";
pub const CONNECTION_LOST: &str = "conexao_perdida";
pub const NOT_CONNECTED: &str = "O WhatsApp não está conectado";
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const SHUTDOWN_WAIT: Duration = Duration::from_millis(1800);
const SESSION_STOP: Duration = Duration::from_millis(1500);
const DEVICE_NAME: &str = "Hub Pessoal";
const MAX_QR_RENEWALS: u32 = 3;
const MAX_UNEXPECTED_ENDS: u32 = 3;
const HISTORY_SEND: usize = 64;
pub const STICKER_SIZE: u32 = 512;
const MAX_STICKER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BridgeEvent {
    Ready(BridgeHandle),
    Qr(String),
    Connected(String),
    Message(WireMessage),
    Edit(WireEdit),
    Revoke(WireRevoke),
    Reaction(WireReaction),
    Contact(WireConversation),
    Disconnected(String),
    Archived(String, bool),
    FavoriteSticker(String, bool, Vec<u8>),
    Error(String),
    Exited(String),
}

type Out = mpsc::UnboundedSender<BridgeEvent>;

struct Shared {
    client: RwLock<Option<Arc<Client>>>,
    data_dir: PathBuf,
    main_db: PathBuf,
    names: Mutex<HashMap<String, String>>,
    stop: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

impl Shared {
    fn set_client(&self, client: Option<Arc<Client>>) {
        *self.client.write().unwrap_or_else(|e| e.into_inner()) = client;
    }

    fn client(&self) -> Option<Arc<Client>> {
        self.client
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn remember(&self, jid: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() || jid.is_empty() {
            return;
        }
        self.names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(jid.to_string(), name.to_string());
    }

    fn name_of(&self, jid: &str) -> Option<String> {
        self.names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(jid)
            .cloned()
    }
}

#[derive(Clone)]
pub struct BridgeHandle {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for BridgeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BridgeHandle")
    }
}

pub fn spawn(data_dir: PathBuf) -> impl Stream<Item = BridgeEvent> + Send + 'static {
    let main_db = data_dir.join(MAIN_DB_FILE);
    spawn_with(data_dir, main_db)
}

pub fn spawn_with(
    data_dir: PathBuf,
    main_db: PathBuf,
) -> impl Stream<Item = BridgeEvent> + Send + 'static {
    let (out, events) = mpsc::unbounded();
    let (stop_tx, stop_rx) = watch::channel(false);
    let (done_tx, done_rx) = watch::channel(false);
    let shared = Arc::new(Shared {
        client: RwLock::new(None),
        data_dir,
        main_db,
        names: Mutex::new(HashMap::new()),
        stop: stop_tx,
        done: done_rx,
    });
    let _ = out.unbounded_send(BridgeEvent::Ready(BridgeHandle {
        shared: shared.clone(),
    }));
    tokio::spawn(supervise(shared, out, stop_rx, done_tx));
    events
}

#[cfg(test)]
pub(crate) fn detached(data_dir: PathBuf, main_db: PathBuf) -> BridgeHandle {
    let (stop_tx, _) = watch::channel(false);
    let (done_tx, done_rx) = watch::channel(true);
    drop(done_tx);
    BridgeHandle {
        shared: Arc::new(Shared {
            client: RwLock::new(None),
            data_dir,
            main_db,
            names: Mutex::new(HashMap::new()),
            stop: stop_tx,
            done: done_rx,
        }),
    }
}

async fn within<T>(
    limit: Duration,
    what: &str,
    work: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    match tokio::time::timeout(limit, work).await {
        Ok(result) => result,
        Err(_) => Err(format!("O WhatsApp não respondeu a tempo ao {what}")),
    }
}

fn failure(what: &str, error: &dyn std::fmt::Display) -> String {
    format!("Não foi possível {what}: {error}")
}

fn send_failure(what: &str, error: &SendError) -> String {
    match error {
        SendError::NotLoggedIn => NOT_CONNECTED.to_string(),
        other => failure(what, other),
    }
}

fn parse_jid(text: &str) -> Result<Jid, String> {
    let jid = Jid::from_str(text.trim()).map_err(|_| "Conversa inválida".to_string())?;
    if jid.user.is_empty() || jid.is_status_broadcast() || jid.is_newsletter() {
        return Err("Esta conversa não aceita envio de mensagens".to_string());
    }
    Ok(jid)
}

fn decode_stored(stored: &StoredRaw) -> Result<wa::Message, String> {
    if !stored.raw.is_empty() {
        return waproto::codec::message_decode(&stored.raw)
            .map_err(|e| failure("ler a mensagem guardada", &e));
    }
    if !stored.legacy_media.is_empty() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&stored.legacy_media)
            .map_err(|e| failure("ler a mídia guardada", &e))?;
        let image = wa::message::ImageMessage::decode_from_slice(&bytes)
            .map_err(|e| failure("ler a mídia guardada", &e))?;
        let mut message = wa::Message::default();
        message.image_message = MessageField::some(image);
        return Ok(message);
    }
    Err("A mensagem não tem conteúdo guardado para esta operação".to_string())
}

fn now_unix() -> i64 {
    Utc::now().timestamp()
}

impl BridgeHandle {
    pub fn is_connected(&self) -> bool {
        self.shared.client().is_some_and(|c| c.is_logged_in())
    }

    pub async fn shutdown(&self) {
        let _ = self.shared.stop.send(true);
        let mut done = self.shared.done.clone();
        let _ = tokio::time::timeout(SHUTDOWN_WAIT, done.wait_for(|finished| *finished)).await;
    }

    fn client(&self) -> Result<Arc<Client>, String> {
        self.shared
            .client()
            .filter(|client| client.is_logged_in())
            .ok_or_else(|| NOT_CONNECTED.to_string())
    }

    fn db(&self) -> Result<Connection, String> {
        let conn = Connection::open(&self.shared.main_db)
            .map_err(|e| failure("abrir o banco de dados do Hub", &e))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| failure("abrir o banco de dados do Hub", &e))?;
        Ok(conn)
    }

    fn stored(&self, jid: &str, id: &str) -> Result<StoredRaw, String> {
        let conn = self.db()?;
        super::raw_message(&conn, jid, id)
            .map_err(|e| failure("consultar a mensagem", &e))?
            .ok_or_else(|| "A mensagem não foi encontrada no Hub".to_string())
    }

    fn convert(&self, client: Arc<Client>) -> Convert {
        Convert {
            client,
            shared: self.shared.clone(),
        }
    }

    async fn own_author(&self, client: &Client, chat: &Jid) -> Option<Jid> {
        if chat.is_group()
            && let Ok(routing) = client.groups().routing_info(chat).await
        {
            use whatsapp_rust::wacore::types::message::AddressingMode;
            return match routing.addressing_mode {
                AddressingMode::Lid => client.lid().or_else(|| client.pn()),
                AddressingMode::Pn => client.pn(),
            };
        }
        client.pn()
    }

    async fn quote_context(
        &self,
        client: &Client,
        jid: &str,
        chat: &Jid,
        reply_to: &str,
    ) -> Result<wa::ContextInfo, String> {
        let stored = self.stored(jid, reply_to)?;
        let message = decode_stored(&stored)?;
        let sender = if stored.mine {
            self.own_author(client, chat).await
        } else {
            let raw = if stored.sender_raw.is_empty() {
                &stored.sender
            } else {
                &stored.sender_raw
            };
            Jid::from_str(raw).ok()
        }
        .ok_or_else(|| "Não foi possível identificar o autor da mensagem respondida".to_string())?;
        Ok(whatsapp_rust::proto_helpers::build_quote_context_with_info(
            reply_to, &sender, chat, chat, &message,
        ))
    }

    async fn sent_wire(
        &self,
        client: &Client,
        jid: &str,
        chat: &Jid,
        result: &whatsapp_rust::SendResult,
    ) -> Result<WireMessage, String> {
        let own = client
            .pn()
            .map(|j| j.to_non_ad_string())
            .unwrap_or_default();
        let author = self
            .own_author(client, chat)
            .await
            .map(|j| j.to_non_ad_string())
            .unwrap_or_else(|| own.clone());
        let meta = Meta {
            id: result.message_id.to_string(),
            chat: jid.to_string(),
            sender: own,
            sender_raw: author,
            from_me: true,
            ts: now_unix(),
            ..Meta::default()
        };
        match convert::build(&meta, &result.message) {
            Content::Message(wire) => Ok(*wire),
            _ => Err("A mensagem foi enviada, mas não foi possível registrá-la".to_string()),
        }
    }

    async fn dispatch(
        &self,
        jid: &str,
        message: wa::Message,
        what: &str,
    ) -> Result<WireMessage, String> {
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let result = client
            .send_message(chat.clone(), message)
            .await
            .map_err(|e| send_failure(what, &e))?;
        self.sent_wire(&client, jid, &chat, &result).await
    }

    pub async fn conversations(&self) -> Result<Vec<WireConversation>, String> {
        let client = self.client()?;
        within(CALL_TIMEOUT, "listar os grupos", async {
            let groups = client
                .groups()
                .list_participating()
                .await
                .map_err(|e| failure("listar os grupos", &e))?;
            Ok(groups
                .into_iter()
                .map(|group| WireConversation {
                    jid: group.id.to_non_ad_string(),
                    nome: group.subject.unwrap_or_default(),
                    grupo: true,
                })
                .collect())
        })
        .await
    }

    pub async fn chat_info(&self, jids: Vec<String>) -> Result<Vec<WireChatInfo>, String> {
        let client = self.client()?;
        let convert = self.convert(client);
        within(CALL_TIMEOUT * 2, "consultar as conversas", async {
            let mut memo = HashMap::new();
            let mut infos = Vec::with_capacity(jids.len());
            for raw in jids {
                let Ok(jid) = Jid::from_str(&raw) else {
                    continue;
                };
                let resolved = convert.resolve(&jid, &mut memo).await;
                let phone = if resolved.is_pn() && jid.is_lid() {
                    resolved.to_string()
                } else {
                    String::new()
                };
                infos.push(WireChatInfo {
                    jid: raw,
                    phone,
                    nome: String::new(),
                    arquivada: None,
                });
            }
            Ok(infos)
        })
        .await
    }

    pub async fn participants(&self, jid: &str) -> Result<Vec<WireParticipant>, String> {
        let client = self.client()?;
        let group = parse_jid(jid)?;
        if !group.is_group() {
            return Err("A conversa não é um grupo".to_string());
        }
        let metadata = within(CALL_TIMEOUT, "listar os participantes", async {
            let mut metadata = client
                .groups()
                .fetch_metadata(&group)
                .await
                .map_err(|e| failure("listar os participantes", &e))?;
            client
                .groups()
                .resolve_participant_addresses(&mut metadata)
                .await;
            Ok(metadata)
        })
        .await?;
        let own_users: Vec<String> = [client.pn(), client.lid()]
            .into_iter()
            .flatten()
            .map(|j| j.user_base().to_string())
            .collect();
        let conn = self.db().ok();
        let mut list = Vec::with_capacity(metadata.participants.len());
        for participant in &metadata.participants {
            let phone = participant.phone_number.as_ref();
            let is_own = own_users.iter().any(|user| {
                user == participant.jid.user_base() || phone.is_some_and(|p| user == p.user_base())
            });
            if is_own {
                continue;
            }
            let known = phone
                .map(|p| p.to_non_ad_string())
                .unwrap_or_else(|| participant.jid.to_non_ad_string());
            let name = conn
                .as_ref()
                .and_then(|conn| super::display_name(conn, &known))
                .or_else(|| self.shared.name_of(&known))
                .or_else(|| super::phone_label(&known))
                .unwrap_or_else(|| participant.jid.user_base().to_string());
            list.push(WireParticipant {
                jid: participant.jid.to_non_ad_string(),
                nome: name,
            });
        }
        Ok(list)
    }

    pub async fn send_text(
        &self,
        jid: &str,
        text: &str,
        mentions: Vec<String>,
        reply_to: Option<&str>,
    ) -> Result<WireMessage, String> {
        if text.trim().is_empty() {
            return Err("O texto está vazio".to_string());
        }
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let mut context = match reply_to {
            Some(id) => Some(self.quote_context(&client, jid, &chat, id).await?),
            None => None,
        };
        if !mentions.is_empty() {
            context
                .get_or_insert_with(wa::ContextInfo::default)
                .mentioned_jid = mentions;
        }
        let message = match context {
            Some(context) => wa::Message::text_with_context(text, context),
            None => wa::Message::text(text),
        };
        within(
            CALL_TIMEOUT,
            "enviar a mensagem",
            self.dispatch(jid, message, "enviar a mensagem"),
        )
        .await
    }

    pub async fn send_file(
        &self,
        jid: &str,
        path: &Path,
        caption: &str,
        reply_to: Option<&str>,
    ) -> Result<WireMessage, String> {
        let owned = path.to_path_buf();
        let (kind, mime) = files::classify_path(&owned);
        let name = owned
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("arquivo")
            .to_string();
        let data = read_limited(owned).await?;
        self.send_media(jid, kind, mime, name, data, caption, reply_to, None)
            .await
    }

    pub async fn send_image_bytes(
        &self,
        jid: &str,
        png: Vec<u8>,
        caption: &str,
        reply_to: Option<&str>,
    ) -> Result<WireMessage, String> {
        if png.is_empty() || png.len() as u64 > files::MAX_FILE_BYTES {
            return Err("A imagem está vazia ou passa de 100 MB".to_string());
        }
        self.send_media(
            jid,
            OutKind::Image,
            "image/png".to_string(),
            "imagem.png".to_string(),
            png,
            caption,
            reply_to,
            None,
        )
        .await
    }

    pub async fn send_voice(
        &self,
        jid: &str,
        path: &Path,
        seconds: u32,
        waveform: Option<Vec<u8>>,
        reply_to: Option<&str>,
    ) -> Result<WireMessage, String> {
        let data = read_limited(path.to_path_buf()).await?;
        self.send_media(
            jid,
            OutKind::Audio,
            "audio/ogg; codecs=opus".to_string(),
            "audio.ogg".to_string(),
            data,
            "",
            reply_to,
            Some(Voice { seconds, waveform }),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_media(
        &self,
        jid: &str,
        kind: OutKind,
        mime: String,
        name: String,
        data: Vec<u8>,
        caption: &str,
        reply_to: Option<&str>,
        voice: Option<Voice>,
    ) -> Result<WireMessage, String> {
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let context = match reply_to {
            Some(id) => Some(self.quote_context(&client, jid, &chat, id).await?),
            None => None,
        };
        let preview = if kind == OutKind::Image {
            let copy = data.clone();
            tokio::task::spawn_blocking(move || image_preview(&copy))
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let media_type = match (kind, voice.is_some()) {
            (OutKind::Image, _) => MediaType::Image,
            (OutKind::Video, _) => MediaType::Video,
            (OutKind::Audio, true) => MediaType::Ptt,
            (OutKind::Audio, false) => MediaType::Audio,
            (OutKind::Document, _) => MediaType::Document,
        };
        let sent = within(UPLOAD_TIMEOUT, "enviar o arquivo", async {
            let upload = client
                .upload(data, media_type, UploadOptions::default())
                .await
                .map_err(|e| failure("enviar o arquivo", &e))?;
            let message = build_media_message(
                kind,
                &mime,
                &name,
                caption,
                &upload,
                preview,
                voice.as_ref(),
                context,
            );
            let result = client
                .send_message(chat.clone(), message)
                .await
                .map_err(|e| send_failure("enviar o arquivo", &e))?;
            self.sent_wire(&client, jid, &chat, &result).await
        })
        .await?;
        Ok(sent)
    }

    pub async fn react(&self, jid: &str, id: &str, emoji: &str) -> Result<WireReaction, String> {
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let stored = self.stored(jid, id)?;
        let participant = if chat.is_group() {
            if stored.mine {
                self.own_author(&client, &chat)
                    .await
                    .map(|j| j.to_non_ad_string())
            } else if stored.sender_raw.is_empty() {
                Some(stored.sender.clone())
            } else {
                Some(stored.sender_raw.clone())
            }
        } else {
            None
        };
        let mut key = wa::MessageKey::default();
        key.remote_jid = Some(chat.to_string());
        key.from_me = Some(stored.mine);
        key.id = Some(id.to_string());
        key.participant = participant;
        within(CALL_TIMEOUT, "reagir à mensagem", async {
            client
                .send_reaction_raw(chat.clone(), key, emoji.trim())
                .await
                .map(|_| ())
                .map_err(|e| send_failure("reagir à mensagem", &e))
        })
        .await?;
        let own = client
            .pn()
            .map(|j| j.to_non_ad_string())
            .unwrap_or_default();
        Ok(WireReaction {
            jid: jid.to_string(),
            id: id.to_string(),
            de: own,
            minha: true,
            emoji: emoji.trim().to_string(),
            em: now_unix(),
        })
    }

    pub async fn edit(&self, jid: &str, id: &str, text: &str) -> Result<WireEdit, String> {
        if text.trim().is_empty() {
            return Err("O texto está vazio".to_string());
        }
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let stored = self.stored(jid, id)?;
        if !stored.mine {
            return Err("Só é possível editar mensagens enviadas por você".to_string());
        }
        let original = decode_stored(&stored)?;
        let content = edited_content(original.get_base_message(), text.trim())?;
        within(CALL_TIMEOUT, "editar a mensagem", async {
            client
                .edit_message_raw(chat.clone(), id, content, EditOptions::default())
                .await
                .map(|_| ())
                .map_err(|e| send_failure("editar a mensagem", &e))
        })
        .await?;
        let own = client
            .pn()
            .map(|j| j.to_non_ad_string())
            .unwrap_or_default();
        Ok(WireEdit {
            jid: jid.to_string(),
            id: id.to_string(),
            de: own,
            minha: true,
            texto: text.trim().to_string(),
            em: now_unix(),
        })
    }

    pub async fn revoke(&self, jid: &str, id: &str) -> Result<WireRevoke, String> {
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let stored = self.stored(jid, id)?;
        if !stored.mine {
            return Err(
                "Só é possível apagar para todos as mensagens enviadas por você".to_string(),
            );
        }
        within(CALL_TIMEOUT, "apagar a mensagem", async {
            client
                .revoke_message_raw(chat.clone(), id, RevokeType::Sender)
                .await
                .map(|_| ())
                .map_err(|e| send_failure("apagar a mensagem", &e))
        })
        .await?;
        let own = client
            .pn()
            .map(|j| j.to_non_ad_string())
            .unwrap_or_default();
        Ok(WireRevoke {
            jid: jid.to_string(),
            id: id.to_string(),
            de: own,
            minha: true,
            admin: false,
            em: now_unix(),
        })
    }

    pub async fn forward(
        &self,
        id: &str,
        origin: &str,
        destinations: Vec<String>,
    ) -> Result<Vec<Result<WireMessage, String>>, String> {
        let client = self.client()?;
        let stored = self.stored(origin, id)?;
        let message = decode_stored(&stored)?;
        let mut results = Vec::with_capacity(destinations.len());
        for destination in destinations {
            let result = async {
                let chat = parse_jid(&destination)?;
                let sent = within(UPLOAD_TIMEOUT, "encaminhar a mensagem", async {
                    client
                        .forward_message(chat.clone(), &message)
                        .await
                        .map_err(|e| send_failure("encaminhar a mensagem", &e))
                })
                .await?;
                self.sent_wire(&client, &destination, &chat, &sent).await
            }
            .await;
            results.push(result);
        }
        Ok(results)
    }

    pub async fn send_sticker(
        &self,
        jid: &str,
        hash: &str,
        action: &[u8],
    ) -> Result<WireMessage, String> {
        let message = sticker_message(hash, action)?;
        within(
            UPLOAD_TIMEOUT,
            "enviar a figurinha",
            self.dispatch(jid, message, "enviar a figurinha"),
        )
        .await
    }

    pub async fn favorite_sticker(
        &self,
        hash: &str,
        action: &[u8],
        favorite: bool,
    ) -> Result<(), String> {
        let client = self.client()?;
        let mut sticker = wa::sync_action_value::StickerAction::decode_from_slice(action)
            .map_err(|e| failure("ler a figurinha", &e))?;
        sticker.is_favorite = Some(favorite);
        let mut value = wa::SyncActionValue::default();
        value.sticker_action = MessageField::some(sticker);
        value.timestamp = Some(Utc::now().timestamp_millis());
        within(CALL_TIMEOUT, "salvar a figurinha", async {
            client
                .send_app_state_action(&schemas::FAVORITE_STICKER, &[hash], &value)
                .await
                .map_err(|e| failure("salvar a figurinha na conta", &e))
        })
        .await
    }

    pub async fn sticker_file(&self, hash: &str, action: &[u8]) -> Result<PathBuf, String> {
        let dir = self.shared.data_dir.join("midia").join("figurinhas");
        let name: String = hash.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let path = dir.join(format!("{name}.webp"));
        if path.is_file() {
            return Ok(path);
        }
        let message = sticker_message(hash, action)?;
        let Some(target) = message.sticker_message.as_option() else {
            return Err("Figurinha inválida".to_string());
        };
        let client = self.client()?;
        std::fs::create_dir_all(&dir).map_err(|e| failure("criar a pasta de figurinhas", &e))?;
        within(
            DOWNLOAD_TIMEOUT,
            "baixar a figurinha",
            save_download(&client, target, &path),
        )
        .await?;
        Ok(path)
    }

    pub async fn send_sticker_image(&self, jid: &str, path: &Path) -> Result<WireMessage, String> {
        let data = read_limited(path.to_path_buf()).await?;
        let webp = tokio::task::spawn_blocking(move || sticker_webp(&data))
            .await
            .map_err(|e| failure("preparar a figurinha", &e))??;
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        within(UPLOAD_TIMEOUT, "enviar a figurinha", async {
            let upload = client
                .upload(webp, MediaType::Sticker, UploadOptions::default())
                .await
                .map_err(|e| failure("enviar a figurinha", &e))?;
            let mut sticker = wa::message::StickerMessage::default();
            sticker.url = Some(upload.url.clone());
            sticker.mimetype = Some("image/webp".to_string());
            sticker.file_sha256 = Some(upload.file_sha256.to_vec());
            sticker.file_length = Some(upload.file_length);
            sticker.media_key = Some(upload.media_key.to_vec());
            sticker.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            sticker.direct_path = Some(upload.direct_path.clone());
            sticker.media_key_timestamp = Some(upload.media_key_timestamp);
            sticker.width = Some(STICKER_SIZE);
            sticker.height = Some(STICKER_SIZE);
            let mut message = wa::Message::default();
            message.sticker_message = MessageField::some(sticker);
            let result = client
                .send_message(chat.clone(), message)
                .await
                .map_err(|e| send_failure("enviar a figurinha", &e))?;
            self.sent_wire(&client, jid, &chat, &result).await
        })
        .await
    }

    pub async fn download_media(&self, jid: &str, id: &str) -> Result<PathBuf, String> {
        let stored = self.stored(jid, id)?;
        if !stored.file.is_empty() && Path::new(&stored.file).is_file() {
            return Ok(PathBuf::from(stored.file));
        }
        if !stored.kind.is_media() {
            return Err("Esta mensagem não tem arquivo para baixar".to_string());
        }
        let message = decode_stored(&stored)?;
        let base = message.get_base_message();
        let client = self.client()?;
        let dir = self.shared.data_dir.join("midia");
        std::fs::create_dir_all(&dir).map_err(|e| failure("criar a pasta de mídia", &e))?;
        let (target, mime, name): (&dyn Downloadable, String, String) =
            if let Some(m) = base.image_message.as_option() {
                (m, m.mimetype.clone().unwrap_or_default(), String::new())
            } else if let Some(m) = base
                .video_message
                .as_option()
                .or_else(|| base.ptv_message.as_option())
            {
                (m, m.mimetype.clone().unwrap_or_default(), String::new())
            } else if let Some(m) = base.audio_message.as_option() {
                (m, m.mimetype.clone().unwrap_or_default(), String::new())
            } else if let Some(m) = base.document_message.as_option() {
                (
                    m,
                    m.mimetype.clone().unwrap_or_default(),
                    m.file_name.clone().unwrap_or_default(),
                )
            } else if let Some(m) = base.sticker_message.as_option() {
                (m, m.mimetype.clone().unwrap_or_default(), String::new())
            } else {
                return Err("Esta mensagem não tem arquivo para baixar".to_string());
            };
        let mime = if mime.is_empty() {
            stored.mime.clone()
        } else {
            mime
        };
        let name = if name.is_empty() {
            stored.file_name.clone()
        } else {
            name
        };
        let path = files::media_path(&dir, id, stored.kind, &mime, &name);
        within(
            DOWNLOAD_TIMEOUT,
            "baixar o arquivo",
            save_download(&client, target, &path),
        )
        .await?;
        let conn = self.db()?;
        super::set_file(&conn, jid, id, &path.to_string_lossy())
            .map_err(|e| failure("registrar o arquivo baixado", &e))?;
        Ok(path)
    }

    pub async fn profile_picture(&self, jid: &str) -> Result<Option<PathBuf>, String> {
        let now = chrono::Local::now().naive_local();
        {
            let conn = self.db()?;
            if let Ok(Some(cached)) = super::photo(&conn, jid, now) {
                match cached {
                    Some(path) if Path::new(&path).is_file() => {
                        return Ok(Some(PathBuf::from(path)));
                    }
                    Some(_) => {}
                    None => return Ok(None),
                }
            }
        }
        let client = self.client()?;
        let target = parse_jid(jid)?;
        let lookup = within(CALL_TIMEOUT, "buscar a foto", async {
            let request = if target.is_group() {
                ProfilePictureRequest::new(
                    ProfilePictureTarget::Group(&target),
                    ProfilePictureType::Preview,
                )
            } else {
                ProfilePictureRequest::new(
                    ProfilePictureTarget::Contact(&target),
                    ProfilePictureType::Preview,
                )
            };
            client
                .pictures()
                .lookup(request)
                .await
                .map_err(|e| failure("buscar a foto", &e))
        })
        .await?;
        let stored_path = match lookup {
            ProfilePictureLookup::Found(picture) => {
                let dir = self.shared.data_dir.join("fotos");
                let path = dir.join(files::photo_file_name(jid));
                let url = picture.url.clone();
                let target_path = path.clone();
                tokio::task::spawn_blocking(move || fetch_picture(&url, &dir, &target_path))
                    .await
                    .map_err(|e| failure("baixar a foto", &e))??;
                path.to_string_lossy().into_owned()
            }
            _ => String::new(),
        };
        let conn = self.db()?;
        super::set_photo(&conn, jid, &stored_path, now)
            .map_err(|e| failure("registrar a foto", &e))?;
        Ok((!stored_path.is_empty()).then(|| PathBuf::from(stored_path)))
    }

    pub async fn request_history(&self, jid: &str, count: u32) -> Result<(), String> {
        let client = self.client()?;
        let chat = parse_jid(jid)?;
        let (id, mine, at) = {
            let conn = self.db()?;
            super::oldest_message(&conn, jid)
                .map_err(|e| failure("consultar o histórico local", &e))?
                .ok_or_else(|| {
                    "Não há mensagens locais para pedir o histórico anterior".to_string()
                })?
        };
        within(CALL_TIMEOUT, "pedir o histórico", async {
            client
                .fetch_message_history(
                    &chat,
                    &id,
                    mine,
                    super::to_unix_millis(at),
                    count.clamp(1, 200) as i32,
                )
                .await
                .map(|_| ())
                .map_err(|e| failure("pedir o histórico ao celular", &e))
        })
        .await
    }
}

struct Voice {
    seconds: u32,
    waveform: Option<Vec<u8>>,
}

async fn read_limited(path: PathBuf) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || {
        let metadata = std::fs::metadata(&path).map_err(|e| failure("ler o arquivo", &e))?;
        if !metadata.is_file() {
            return Err("O caminho informado não é um arquivo".to_string());
        }
        if metadata.len() > files::MAX_FILE_BYTES {
            return Err("O arquivo passa de 100 MB".to_string());
        }
        if metadata.len() == 0 {
            return Err("O arquivo está vazio".to_string());
        }
        std::fs::read(&path).map_err(|e| failure("ler o arquivo", &e))
    })
    .await
    .map_err(|e| failure("ler o arquivo", &e))?
}

pub fn sticker_action(raw: &[u8]) -> Option<(String, Vec<u8>)> {
    let message = waproto::codec::message_decode(raw).ok()?;
    let sticker = message.get_base_message().sticker_message.as_option()?;
    let hash = base64::engine::general_purpose::STANDARD.encode(sticker.file_sha256.as_ref()?);
    let mut action = wa::sync_action_value::StickerAction::default();
    action.url = sticker.url.clone();
    action.file_enc_sha256 = sticker.file_enc_sha256.clone();
    action.media_key = sticker.media_key.clone();
    action.mimetype = sticker.mimetype.clone();
    action.height = sticker.height;
    action.width = sticker.width;
    action.direct_path = sticker.direct_path.clone();
    action.file_length = sticker.file_length;
    Some((hash, action.encode_to_vec()))
}

fn sticker_message(hash: &str, action: &[u8]) -> Result<wa::Message, String> {
    let action = wa::sync_action_value::StickerAction::decode_from_slice(action)
        .map_err(|e| failure("ler a figurinha", &e))?;
    let file_sha256 = base64::engine::general_purpose::STANDARD
        .decode(hash)
        .map_err(|e| failure("ler a figurinha", &e))?;
    let mut sticker = wa::message::StickerMessage::default();
    sticker.url = action.url;
    sticker.file_sha256 = Some(file_sha256);
    sticker.file_enc_sha256 = action.file_enc_sha256;
    sticker.media_key = action.media_key;
    sticker.mimetype = action.mimetype.or_else(|| Some("image/webp".to_string()));
    sticker.height = action.height;
    sticker.width = action.width;
    sticker.direct_path = action.direct_path;
    sticker.file_length = action.file_length;
    let mut message = wa::Message::default();
    message.sticker_message = MessageField::some(sticker);
    Ok(message)
}

pub fn video_thumbnail(conn: &Connection, jid: &str, id: &str) -> Option<Vec<u8>> {
    let stored = super::raw_message(conn, jid, id).ok()??;
    let message = decode_stored(&stored).ok()?;
    let base = message.get_base_message();
    base.video_message
        .as_option()
        .or_else(|| base.ptv_message.as_option())?
        .jpeg_thumbnail
        .clone()
        .filter(|thumbnail| !thumbnail.is_empty())
}

pub(crate) fn sticker_webp(data: &[u8]) -> Result<Vec<u8>, String> {
    let source = image::load_from_memory(data).map_err(|e| failure("ler a imagem", &e))?;
    let fitted = source
        .resize(
            STICKER_SIZE,
            STICKER_SIZE,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgba8();
    let mut canvas = image::RgbaImage::new(STICKER_SIZE, STICKER_SIZE);
    let x = (STICKER_SIZE - fitted.width()) / 2;
    let y = (STICKER_SIZE - fitted.height()) / 2;
    image::imageops::overlay(&mut canvas, &fitted, i64::from(x), i64::from(y));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::WebP,
        )
        .map_err(|e| failure("preparar a figurinha", &e))?;
    if out.len() > MAX_STICKER_BYTES {
        return Err("A figurinha passou de 1 MB. Use uma imagem mais simples.".to_string());
    }
    Ok(out)
}

struct Preview {
    width: u32,
    height: u32,
    thumbnail: Vec<u8>,
}

fn image_preview(data: &[u8]) -> Option<Preview> {
    let image = image::load_from_memory(data).ok()?;
    let (width, height) = (image.width(), image.height());
    let small = image::DynamicImage::ImageRgb8(image.thumbnail(100, 100).to_rgb8());
    let mut thumbnail = Vec::new();
    small
        .write_to(
            &mut std::io::Cursor::new(&mut thumbnail),
            image::ImageFormat::Jpeg,
        )
        .ok()?;
    Some(Preview {
        width,
        height,
        thumbnail,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_media_message(
    kind: OutKind,
    mime: &str,
    name: &str,
    caption: &str,
    upload: &whatsapp_rust::upload::UploadResponse,
    preview: Option<Preview>,
    voice: Option<&Voice>,
    context: Option<wa::ContextInfo>,
) -> wa::Message {
    let context = match context {
        Some(context) => MessageField::some(context),
        None => MessageField::none(),
    };
    let caption = (!caption.trim().is_empty()).then(|| caption.trim().to_string());
    let mut message = wa::Message::default();
    match kind {
        OutKind::Image => {
            let mut image = wa::message::ImageMessage::default();
            image.url = Some(upload.url.clone());
            image.mimetype = Some(mime.to_string());
            image.caption = caption;
            image.file_sha256 = Some(upload.file_sha256.to_vec());
            image.file_length = Some(upload.file_length);
            image.media_key = Some(upload.media_key.to_vec());
            image.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            image.direct_path = Some(upload.direct_path.clone());
            image.media_key_timestamp = Some(upload.media_key_timestamp);
            if let Some(preview) = preview {
                image.width = Some(preview.width);
                image.height = Some(preview.height);
                image.jpeg_thumbnail = Some(preview.thumbnail);
            }
            image.context_info = context;
            message.image_message = MessageField::some(image);
        }
        OutKind::Video => {
            let mut video = wa::message::VideoMessage::default();
            video.url = Some(upload.url.clone());
            video.mimetype = Some(mime.to_string());
            video.caption = caption;
            video.file_sha256 = Some(upload.file_sha256.to_vec());
            video.file_length = Some(upload.file_length);
            video.media_key = Some(upload.media_key.to_vec());
            video.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            video.direct_path = Some(upload.direct_path.clone());
            video.media_key_timestamp = Some(upload.media_key_timestamp);
            video.context_info = context;
            message.video_message = MessageField::some(video);
        }
        OutKind::Audio => {
            let mut audio = wa::message::AudioMessage::default();
            audio.url = Some(upload.url.clone());
            audio.mimetype = Some(mime.to_string());
            audio.file_sha256 = Some(upload.file_sha256.to_vec());
            audio.file_length = Some(upload.file_length);
            audio.media_key = Some(upload.media_key.to_vec());
            audio.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            audio.direct_path = Some(upload.direct_path.clone());
            audio.media_key_timestamp = Some(upload.media_key_timestamp);
            if let Some(voice) = voice {
                audio.ptt = Some(true);
                audio.seconds = Some(voice.seconds);
                audio.waveform = voice.waveform.clone();
            }
            audio.context_info = context;
            message.audio_message = MessageField::some(audio);
        }
        OutKind::Document => {
            let mut document = wa::message::DocumentMessage::default();
            document.url = Some(upload.url.clone());
            document.mimetype = Some(mime.to_string());
            document.title = Some(name.to_string());
            document.file_name = Some(name.to_string());
            document.caption = caption;
            document.file_sha256 = Some(upload.file_sha256.to_vec());
            document.file_length = Some(upload.file_length);
            document.media_key = Some(upload.media_key.to_vec());
            document.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            document.direct_path = Some(upload.direct_path.clone());
            document.media_key_timestamp = Some(upload.media_key_timestamp);
            document.context_info = context;
            message.document_message = MessageField::some(document);
        }
    }
    message
}

fn edited_content(original: &wa::Message, text: &str) -> Result<wa::Message, String> {
    if original.image_message.is_set() || original.video_message.is_set() {
        return original
            .media_with_caption(text)
            .ok_or_else(|| "Esta mensagem não pode ser editada".to_string());
    }
    if let Some(document) = original.document_message.as_option() {
        let mut document = document.clone();
        document.caption = Some(text.to_string());
        let mut message = wa::Message::default();
        message.document_message = MessageField::some(document);
        return Ok(message);
    }
    if original.conversation.is_some() || original.extended_text_message.is_set() {
        return Ok(wa::Message::text(text));
    }
    Err("Esta mensagem não pode ser editada".to_string())
}

async fn save_download(
    client: &Client,
    target: &dyn Downloadable,
    path: &Path,
) -> Result<(), String> {
    let mut partial = path.as_os_str().to_os_string();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let file =
        std::fs::File::create(&partial).map_err(|e| failure("criar o arquivo de destino", &e))?;
    match client
        .download_to_writer(target, file, DownloadLimits::default())
        .await
    {
        Ok(file) => {
            let finished = file
                .sync_all()
                .and_then(|_| {
                    drop(file);
                    std::fs::rename(&partial, path)
                })
                .map_err(|e| failure("gravar o arquivo baixado", &e));
            if finished.is_err() {
                let _ = std::fs::remove_file(&partial);
            }
            finished
        }
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(download_failure(&error))
        }
    }
}

fn download_failure(error: &whatsapp_rust::download::ClientDownloadError) -> String {
    use whatsapp_rust::download::ClientDownloadError as E;
    match error {
        E::ReferenceRejected(_) | E::HostsUnreachable(_) => {
            "O arquivo não está mais disponível no servidor do WhatsApp".to_string()
        }
        other => failure("baixar o arquivo", other),
    }
}

fn fetch_picture(url: &str, dir: &Path, target: &Path) -> Result<(), String> {
    if !files::is_trusted_picture_url(url) {
        return Err("O endereço da foto não é confiável".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| failure("criar a pasta de fotos", &e))?;
    let mut response = ureq::get(url)
        .config()
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .call()
        .map_err(|e| failure("baixar a foto", &e))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(files::MAX_PICTURE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| failure("baixar a foto", &e))?;
    if bytes.is_empty() {
        return Err("A foto veio vazia".to_string());
    }
    let mut partial = target.as_os_str().to_os_string();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, &bytes).map_err(|e| failure("gravar a foto", &e))?;
    std::fs::rename(&partial, target).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        failure("gravar a foto", &e)
    })
}

struct Convert {
    client: Arc<Client>,
    shared: Arc<Shared>,
}

impl Convert {
    async fn resolve(&self, jid: &Jid, memo: &mut HashMap<String, Jid>) -> Jid {
        let bare = jid.to_non_ad();
        if !bare.is_lid() {
            return bare;
        }
        let key = bare.to_string();
        if let Some(found) = memo.get(&key) {
            return found.clone();
        }
        let resolved = match self.client.get_lid_pn_entry(&bare).await {
            Ok(Some(entry)) => Jid::pn(&*entry.phone_number),
            _ => bare,
        };
        memo.insert(key, resolved.clone());
        resolved
    }

    async fn resolve_text(&self, raw: &str, memo: &mut HashMap<String, Jid>) -> String {
        match Jid::from_str(raw) {
            Ok(jid) => self.resolve(&jid, memo).await.to_string(),
            Err(_) => raw.to_string(),
        }
    }

    fn own_pn(&self) -> String {
        self.client
            .pn()
            .map(|j| j.to_non_ad_string())
            .unwrap_or_default()
    }

    async fn finish_meta(
        &self,
        mut meta: Meta,
        message: &wa::Message,
        memo: &mut HashMap<String, Jid>,
    ) -> Meta {
        let linked = convert::links(message);
        if !linked.quoted_participant.is_empty() {
            meta.quoted_from = self.resolve_text(&linked.quoted_participant, memo).await;
        }
        let own_users: Vec<String> = [self.client.pn(), self.client.lid()]
            .into_iter()
            .flatten()
            .map(|j| j.user_base().to_string())
            .collect();
        for raw in &linked.mentioned {
            let Ok(jid) = Jid::from_str(raw) else {
                continue;
            };
            let token = jid.user_base().to_string();
            if own_users.contains(&token) {
                meta.own_tokens.push(token);
                continue;
            }
            let resolved = self.resolve(&jid, memo).await.to_string();
            let name = self.shared.name_of(&resolved).unwrap_or_default();
            meta.mentions.push(WireMention {
                token,
                jid: resolved,
                name,
            });
        }
        if !meta.from_me && !meta.push_name.is_empty() {
            self.shared.remember(&meta.sender, &meta.push_name);
        }
        meta
    }

    async fn live_meta(&self, inbound: &InboundMessage, memo: &mut HashMap<String, Jid>) -> Meta {
        let info = &inbound.info;
        let source = &info.source;
        let mut chat = self.resolve(&source.chat, memo).await;
        if chat.is_lid() && !source.is_group {
            let alternative = if source.is_from_me {
                source.recipient_alt.as_ref()
            } else {
                source.sender_alt.as_ref()
            };
            if let Some(alternative) = alternative.filter(|alt| alt.is_pn()) {
                chat = alternative.to_non_ad();
            }
        }
        let sender = if source.is_from_me {
            self.own_pn()
        } else {
            let mut sender = self.resolve(&source.sender, memo).await;
            if sender.is_lid()
                && let Some(alternative) = source.sender_alt.as_ref().filter(|alt| alt.is_pn())
            {
                sender = alternative.to_non_ad();
            }
            sender.to_string()
        };
        let meta = Meta {
            id: info.id.to_string(),
            chat: chat.to_string(),
            sender,
            sender_raw: source.sender.to_non_ad_string(),
            from_me: source.is_from_me,
            push_name: info.push_name.to_string(),
            ts: info.timestamp.as_secs(),
            admin_revoke: info.edit == EditAttribute::AdminRevoke,
            ..Meta::default()
        };
        self.finish_meta(meta, &inbound.message, memo).await
    }

    async fn history_events(&self, history: History, out: &Out) {
        let mut memo = HashMap::new();
        let mut events = Vec::new();
        for (raw, name) in &history.pushnames {
            let resolved = self.resolve_text(raw, &mut memo).await;
            self.shared.remember(&resolved, name);
        }
        for contact in history.contacts {
            let jid = self.resolve_text(&contact.jid, &mut memo).await;
            events.push(BridgeEvent::Contact(WireConversation { jid, ..contact }));
        }
        for (raw, archived) in history.archived {
            let jid = self.resolve_text(&raw, &mut memo).await;
            events.push(BridgeEvent::Archived(jid, archived));
        }
        for row in history.rows {
            let mut chat = self.resolve_text(&row.chat, &mut memo).await;
            if chat.ends_with("@lid")
                && let Some(alternative) = row
                    .alt_chat
                    .as_ref()
                    .filter(|alt| alt.ends_with("@s.whatsapp.net"))
            {
                chat = alternative.clone();
            }
            let sender_raw = if row.from_me {
                String::new()
            } else if row.is_group {
                row.participant.clone()
            } else {
                row.chat.clone()
            };
            let sender = if row.from_me {
                self.own_pn()
            } else {
                self.resolve_text(&sender_raw, &mut memo).await
            };
            let push_name = if row.push_name.is_empty() {
                self.shared.name_of(&sender).unwrap_or_default()
            } else {
                row.push_name.clone()
            };
            let meta = Meta {
                id: row.id.clone(),
                chat,
                sender,
                sender_raw: if row.from_me {
                    self.own_pn()
                } else {
                    sender_raw
                },
                from_me: row.from_me,
                push_name,
                ts: row.ts,
                history: true,
                ..Meta::default()
            };
            let meta = self.finish_meta(meta, &row.message, &mut memo).await;
            push_content(&mut events, convert::build(&meta, &row.message));
            if events.len() >= HISTORY_SEND {
                send_all(out, &mut events);
                tokio::task::yield_now().await;
            }
        }
        send_all(out, &mut events);
    }

    async fn history_worker(self, mut rx: mpsc::UnboundedReceiver<Arc<Event>>, out: Out) {
        while let Some(event) = rx.next().await {
            let extracted = tokio::task::spawn_blocking(move || match &*event {
                Event::HistorySync(lazy) => lazy.get().map(extract_history),
                _ => None,
            })
            .await;
            if let Ok(Some(history)) = extracted {
                self.history_events(history, &out).await;
            }
        }
    }

    async fn handle(&self, event: &Event, state: &mut State) -> Vec<BridgeEvent> {
        let mut events = Vec::new();
        match event {
            Event::Connected(_) => {
                state.connected = true;
                let own = self.own_pn();
                if !own.is_empty() {
                    events.push(BridgeEvent::Connected(own));
                }
            }
            Event::Disconnected(_) => {
                if !state.quiet {
                    events.push(BridgeEvent::Disconnected(CONNECTION_LOST.to_string()));
                }
            }
            Event::PairingQrCode(qr) => events.push(BridgeEvent::Qr(qr.code.clone())),
            Event::PairingQrCodesExhausted(_) => state.qr_expired = true,
            Event::ClientOutdated(_) => state.terminal = Some(Terminal::Outdated),
            Event::LoggedOut(logged_out) => {
                state.terminal = Some(
                    if logged_out.reason == ConnectFailureReason::AccountLocked {
                        Terminal::Locked
                    } else {
                        Terminal::LoggedOut
                    },
                );
            }
            Event::TemporaryBan(ban) => {
                state.terminal = Some(Terminal::Ban(ban_message(&ban.code, ban.expire)));
            }
            Event::ConnectFailure(failure) => match failure.reason {
                ConnectFailureReason::TempBanned => {
                    state.terminal = Some(Terminal::Ban(
                        "O WhatsApp suspendeu temporariamente esta conta. Abra o WhatsApp no celular para ver o prazo; o Hub não vai tentar reconectar."
                            .to_string(),
                    ));
                }
                ConnectFailureReason::AccountLocked => state.terminal = Some(Terminal::Locked),
                ConnectFailureReason::ClientOutdated => state.terminal = Some(Terminal::Outdated),
                reason if reason.is_logged_out() => state.terminal = Some(Terminal::LoggedOut),
                _ => {}
            },
            Event::Messages(batch) => {
                let mut memo = HashMap::new();
                for inbound in batch.iter() {
                    let source = &inbound.info.source;
                    if inbound.info.is_status_stanza
                        || source.chat.is_status_broadcast()
                        || source.chat.is_broadcast_list()
                        || source.chat.is_newsletter()
                    {
                        continue;
                    }
                    let meta = self.live_meta(inbound, &mut memo).await;
                    push_content(&mut events, convert::build(&meta, &inbound.message));
                }
            }
            Event::ArchiveUpdate(update) => {
                let mut memo = HashMap::new();
                let jid = self.resolve(&update.jid, &mut memo).await.to_string();
                events.push(BridgeEvent::Archived(
                    jid,
                    update.action.archived.unwrap_or(false),
                ));
            }
            Event::FavoriteStickerUpdate(update) => {
                events.push(BridgeEvent::FavoriteSticker(
                    update.filehash.clone(),
                    update.action.is_favorite.unwrap_or(false),
                    update.action.encode_to_vec(),
                ));
            }
            Event::ContactUpdate(update) => {
                let name = update
                    .action
                    .full_name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| update.action.first_name.clone())
                    .unwrap_or_default();
                if !name.trim().is_empty() {
                    let mut memo = HashMap::new();
                    let raw = update
                        .action
                        .pn_jid
                        .clone()
                        .unwrap_or_else(|| update.jid.to_string());
                    let jid = self.resolve_text(&raw, &mut memo).await;
                    if jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid") {
                        events.push(BridgeEvent::Contact(WireConversation {
                            jid,
                            nome: name.trim().to_string(),
                            grupo: false,
                        }));
                    }
                }
            }
            _ => {}
        }
        events
    }
}

fn send_all(out: &Out, events: &mut Vec<BridgeEvent>) {
    for event in events.drain(..) {
        let _ = out.unbounded_send(event);
    }
}

fn push_content(events: &mut Vec<BridgeEvent>, content: Content) {
    match content {
        Content::Message(wire) => events.push(BridgeEvent::Message(*wire)),
        Content::Edit(edit) => events.push(BridgeEvent::Edit(edit)),
        Content::Revoke(revoke) => events.push(BridgeEvent::Revoke(revoke)),
        Content::Reaction(reaction) => events.push(BridgeEvent::Reaction(reaction)),
        Content::Skip => {}
    }
}

struct HistoryRow {
    chat: String,
    alt_chat: Option<String>,
    is_group: bool,
    from_me: bool,
    id: String,
    participant: String,
    push_name: String,
    ts: i64,
    message: wa::Message,
}

#[derive(Default)]
struct History {
    rows: Vec<HistoryRow>,
    archived: Vec<(String, bool)>,
    contacts: Vec<WireConversation>,
    pushnames: Vec<(String, String)>,
}

fn extract_history(history: &wa::HistorySync) -> History {
    let mut out = History::default();
    for pushname in &history.pushnames {
        if let (Some(id), Some(name)) = (&pushname.id, &pushname.pushname) {
            out.pushnames.push((id.clone(), name.clone()));
        }
    }
    for conversation in &history.conversations {
        let chat = conversation.id.clone();
        if chat.ends_with("@broadcast") || chat.ends_with("@newsletter") || chat.is_empty() {
            continue;
        }
        let is_group = chat.ends_with("@g.us");
        if let Some(archived) = conversation.archived {
            out.archived.push((chat.clone(), archived));
        }
        let name = conversation
            .name
            .clone()
            .or_else(|| conversation.display_name.clone())
            .unwrap_or_default();
        if !name.trim().is_empty() {
            out.contacts.push(WireConversation {
                jid: chat.clone(),
                nome: name,
                grupo: is_group,
            });
        }
        let alt_chat = conversation.pn_jid.clone();
        for item in &conversation.messages {
            let Some(web) = item.message.as_option() else {
                continue;
            };
            let (Some(key), Some(message)) = (web.key.as_option(), web.message.as_option()) else {
                continue;
            };
            let Some(id) = key.id.clone() else {
                continue;
            };
            let participant = web
                .participant
                .clone()
                .or_else(|| key.participant.clone())
                .unwrap_or_default();
            out.rows.push(HistoryRow {
                chat: chat.clone(),
                alt_chat: alt_chat.clone(),
                is_group,
                from_me: key.from_me.unwrap_or(false),
                id,
                participant,
                push_name: web.push_name.clone().unwrap_or_default(),
                ts: web.message_timestamp.unwrap_or(0) as i64,
                message: message.clone(),
            });
        }
    }
    out
}

#[derive(Default)]
struct State {
    terminal: Option<Terminal>,
    qr_expired: bool,
    connected: bool,
    quiet: bool,
}

enum Terminal {
    LoggedOut,
    Locked,
    Ban(String),
    Outdated,
}

pub(crate) fn human_duration(duration: Duration) -> String {
    let minutes = duration.as_secs().div_ceil(60).max(1);
    if minutes < 60 {
        return if minutes == 1 {
            "1 minuto".to_string()
        } else {
            format!("{minutes} minutos")
        };
    }
    let hours = minutes.div_ceil(60);
    if hours < 48 {
        return if hours == 1 {
            "1 hora".to_string()
        } else {
            format!("{hours} horas")
        };
    }
    format!("{} dias", hours.div_ceil(24))
}

pub(crate) fn ban_message(code: &TempBanReason, expire: Duration) -> String {
    let reason = match code {
        TempBanReason::SentToTooManyPeople => {
            "mensagens enviadas a muitas pessoas que não têm você nos contatos"
        }
        TempBanReason::BlockedByUsers => "muitas pessoas bloquearam você",
        TempBanReason::CreatedTooManyGroups => "muitos grupos criados",
        TempBanReason::SentTooManySameMessage => "a mesma mensagem enviada a muitas pessoas",
        TempBanReason::BroadcastList => "muitas mensagens para uma lista de transmissão",
        _ => "possível violação dos termos de uso",
    };
    format!(
        "O WhatsApp suspendeu temporariamente esta conta ({reason}). A suspensão dura cerca de {}. O Hub não vai tentar reconectar sozinho.",
        human_duration(expire)
    )
}

const LOCKED_MESSAGE: &str = "O WhatsApp bloqueou esta conta. Abra o WhatsApp no celular para ver o motivo e recorrer. O Hub não vai tentar reconectar.";
const OUTDATED_MESSAGE: &str = "O WhatsApp recusou esta versão do Hub por estar desatualizada. Atualize o Hub para voltar a conectar.";

struct Forward {
    tx: mpsc::UnboundedSender<Arc<Event>>,
}

impl EventHandler for Forward {
    fn handle_event(&self, event: Arc<Event>) {
        let _ = self.tx.unbounded_send(event);
    }

    fn interest(&self) -> EventInterest {
        EventInterest::of(&[
            EventKind::Connected,
            EventKind::Disconnected,
            EventKind::LoggedOut,
            EventKind::PairingQrCode,
            EventKind::PairingQrCodesExhausted,
            EventKind::ClientOutdated,
            EventKind::Messages,
            EventKind::HistorySync,
            EventKind::ArchiveUpdate,
            EventKind::ContactUpdate,
            EventKind::TemporaryBan,
            EventKind::ConnectFailure,
            EventKind::FavoriteStickerUpdate,
        ])
    }
}

struct Session {
    handle: BotHandle,
    client: Arc<Client>,
    rx: mpsc::UnboundedReceiver<Arc<Event>>,
}

async fn start_session(db: &SqliteDatabase) -> Result<Session, String> {
    let (tx, rx) = mpsc::unbounded();
    let bot = Bot::builder()
        .with_backend(db.store(1))
        .with_presence_policy(PresencePolicy::Manual)
        .with_device_props(DevicePropsOverride::new().with_os(DEVICE_NAME))
        .with_event_handler(Forward { tx })
        .build()
        .await
        .map_err(|e| failure("preparar o WhatsApp", &e))?;
    let client = bot.client();
    let handle = bot.spawn();
    Ok(Session { handle, client, rx })
}

enum End {
    Stop,
    RunEnded,
    Terminal,
}

struct HistoryTask(tokio::task::JoinHandle<()>);

impl HistoryTask {
    fn stop(&self) -> End {
        self.0.abort();
        End::Stop
    }
}

async fn pump(
    shared: &Arc<Shared>,
    out: &Out,
    stop: &mut watch::Receiver<bool>,
    session: &mut Session,
    state: &mut State,
) -> End {
    let convert = Convert {
        client: session.client.clone(),
        shared: shared.clone(),
    };
    let (history_tx, history_rx) = mpsc::unbounded();
    let worker = Convert {
        client: session.client.clone(),
        shared: shared.clone(),
    };
    let history = HistoryTask(tokio::spawn(worker.history_worker(history_rx, out.clone())));
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => return history.stop(),
            _ = &mut session.handle => {
                while let Ok(event) = session.rx.try_recv() {
                    if matches!(&*event, Event::HistorySync(_)) {
                        let _ = history_tx.unbounded_send(event);
                        continue;
                    }
                    for produced in convert.handle(&event, state).await {
                        let _ = out.unbounded_send(produced);
                    }
                }
                return End::RunEnded;
            }
            received = session.rx.next() => {
                let Some(event) = received else {
                    return End::RunEnded;
                };
                if matches!(&*event, Event::HistorySync(_)) {
                    let _ = history_tx.unbounded_send(event);
                    continue;
                }
                let produced = tokio::select! {
                    biased;
                    _ = stop.changed() => return history.stop(),
                    produced = convert.handle(&event, state) => produced,
                };
                for item in produced {
                    if out.unbounded_send(item).is_err() {
                        return history.stop();
                    }
                }
                if state.terminal.is_some() {
                    return End::Terminal;
                }
            }
        }
    }
}

async fn finish_session(session: Session, run_ended: bool) {
    if run_ended {
        return;
    }
    let _ = tokio::time::timeout(SESSION_STOP, session.handle.shutdown()).await;
}

async fn pause(stop: &mut watch::Receiver<bool>, duration: Duration) -> bool {
    tokio::select! {
        _ = stop.changed() => true,
        _ = tokio::time::sleep(duration) => *stop.borrow(),
    }
}

async fn run_sessions(
    shared: &Arc<Shared>,
    out: &Out,
    stop: &mut watch::Receiver<bool>,
) -> Option<String> {
    if let Err(error) = std::fs::create_dir_all(&shared.data_dir) {
        return Some(failure("criar a pasta de dados", &error));
    }
    let url = shared
        .data_dir
        .join(SESSION_FILE)
        .to_string_lossy()
        .into_owned();
    let db = match SqliteDatabase::open(&url, SqliteDatabaseConfig::default()).await {
        Ok(db) => db,
        Err(error) => return Some(failure("abrir a sessão do WhatsApp", &error)),
    };
    let mut unexpected = 0u32;
    let mut renewals = 0u32;
    loop {
        if *stop.borrow() {
            return None;
        }
        let mut session = match start_session(&db).await {
            Ok(session) => session,
            Err(error) => {
                unexpected += 1;
                if unexpected >= MAX_UNEXPECTED_ENDS {
                    return Some(error);
                }
                if pause(stop, Duration::from_secs(3 * u64::from(unexpected))).await {
                    return None;
                }
                continue;
            }
        };
        shared.set_client(Some(session.client.clone()));
        let mut state = State::default();
        let end = pump(shared, out, stop, &mut session, &mut state).await;
        state.quiet = true;
        shared.set_client(None);
        finish_session(session, matches!(end, End::RunEnded)).await;
        if state.connected {
            unexpected = 0;
            renewals = 0;
        }
        if matches!(end, End::Stop) {
            return None;
        }
        match state.terminal {
            Some(Terminal::Ban(message)) => return Some(message),
            Some(Terminal::Locked) => return Some(LOCKED_MESSAGE.to_string()),
            Some(Terminal::Outdated) => return Some(OUTDATED_MESSAGE.to_string()),
            Some(Terminal::LoggedOut) => {
                if let Err(error) = reset_session(&db).await {
                    return Some(error);
                }
                unexpected = 0;
                renewals = 0;
                let _ = out.unbounded_send(BridgeEvent::Disconnected(SESSION_ENDED.to_string()));
                continue;
            }
            None => {}
        }
        if state.qr_expired && !state.connected {
            renewals += 1;
            if renewals > MAX_QR_RENEWALS {
                return Some(
                    "O código QR expirou. Escolha conectar de novo para gerar outro.".to_string(),
                );
            }
            continue;
        }
        unexpected += 1;
        if unexpected >= MAX_UNEXPECTED_ENDS {
            return Some(
                "A conexão com o WhatsApp foi encerrada várias vezes seguidas. Tente conectar de novo."
                    .to_string(),
            );
        }
        let _ = out.unbounded_send(BridgeEvent::Disconnected(CONNECTION_LOST.to_string()));
        if pause(stop, Duration::from_secs(3 * u64::from(unexpected))).await {
            return None;
        }
    }
}

async fn reset_session(db: &SqliteDatabase) -> Result<(), String> {
    match db.device_exists(1).await {
        Ok(true) => db
            .reset_device(1)
            .await
            .map(|_| ())
            .map_err(|e| failure("reiniciar a sessão do WhatsApp", &e)),
        Ok(false) => Ok(()),
        Err(error) => Err(failure("reiniciar a sessão do WhatsApp", &error)),
    }
}

async fn supervise(
    shared: Arc<Shared>,
    out: Out,
    mut stop: watch::Receiver<bool>,
    done: watch::Sender<bool>,
) {
    let reason = run_sessions(&shared, &out, &mut stop).await;
    shared.set_client(None);
    let _ = out.unbounded_send(BridgeEvent::Exited(
        reason.unwrap_or_else(|| "WhatsApp encerrado".to_string()),
    ));
    let _ = done.send(true);
}
