pub mod bridge;
pub mod protocol;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::platform::CommandSpec;
pub use bridge::BridgeHandle;
pub use protocol::{WireConversation, WireMessage};

pub const SIDECAR_NAME: &str = "whatsapp-bridge";
pub const MESSAGES_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Text,
    Image,
    Audio,
    Video,
    Document,
    Sticker,
    Other,
}

impl MessageKind {
    pub fn parse(text: &str) -> Self {
        match text {
            "texto" => MessageKind::Text,
            "imagem" => MessageKind::Image,
            "audio" => MessageKind::Audio,
            "video" => MessageKind::Video,
            "documento" => MessageKind::Document,
            "figurinha" => MessageKind::Sticker,
            _ => MessageKind::Other,
        }
    }

    pub fn placeholder(self) -> Option<&'static str> {
        match self {
            MessageKind::Text => None,
            MessageKind::Image => Some("[imagem]"),
            MessageKind::Audio => Some("[áudio]"),
            MessageKind::Video => Some("[vídeo]"),
            MessageKind::Document => Some("[documento]"),
            MessageKind::Sticker => Some("[figurinha]"),
            MessageKind::Other => Some("[mensagem não suportada]"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub jid: String,
    pub sender: String,
    pub sender_name: String,
    pub text: String,
    pub mine: bool,
    pub at: NaiveDateTime,
    pub kind: MessageKind,
}

impl Message {
    pub fn display_text(&self) -> String {
        match (self.kind.placeholder(), self.text.trim().is_empty()) {
            (Some(placeholder), true) => placeholder.to_string(),
            (Some(placeholder), false) => format!("{placeholder} {}", self.text),
            (None, _) => self.text.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub jid: String,
    pub name: String,
    pub alias: Option<String>,
    pub group: bool,
    pub last_at: Option<NaiveDateTime>,
    pub preview: Option<String>,
}

impl Conversation {
    pub fn title(&self) -> &str {
        match &self.alias {
            Some(alias) => alias,
            None if !self.name.trim().is_empty() => &self.name,
            None => self.jid.split('@').next().unwrap_or(&self.jid),
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.title().to_lowercase().contains(&query)
            || self.name.to_lowercase().contains(&query)
            || self.jid.contains(&query)
    }
}

pub fn from_unix(seconds: i64) -> NaiveDateTime {
    DateTime::from_timestamp(seconds, 0)
        .map(|utc| utc.with_timezone(&Local).naive_local())
        .unwrap_or_default()
}

pub fn wire_to_message(wire: &WireMessage) -> Message {
    Message {
        id: wire.id.clone(),
        jid: wire.jid.clone(),
        sender: wire.de.clone(),
        sender_name: wire.nome_de.clone(),
        text: wire.texto.clone(),
        mine: wire.minha,
        at: from_unix(wire.em),
        kind: MessageKind::parse(&wire.tipo),
    }
}

fn kind_text(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Text => "texto",
        MessageKind::Image => "imagem",
        MessageKind::Audio => "audio",
        MessageKind::Video => "video",
        MessageKind::Document => "documento",
        MessageKind::Sticker => "figurinha",
        MessageKind::Other => "outro",
    }
}

pub fn upsert_conversation(
    conn: &Connection,
    conversation: &WireConversation,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO wa_conversas (jid, nome, grupo) VALUES (?1, ?2, ?3)
         ON CONFLICT(jid) DO UPDATE SET
            nome = CASE WHEN excluded.nome <> '' THEN excluded.nome ELSE wa_conversas.nome END,
            grupo = excluded.grupo",
        params![
            conversation.jid,
            conversation.nome.trim(),
            conversation.grupo
        ],
    )?;
    Ok(())
}

pub fn store_message(conn: &Connection, message: &Message) -> rusqlite::Result<bool> {
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO wa_mensagens (id, jid, de, nome_de, texto, minha, em, tipo)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id,
            message.jid,
            message.sender,
            message.sender_name,
            message.text,
            message.mine,
            message.at,
            kind_text(message.kind)
        ],
    )? > 0;
    let fallback_name = if message.mine || message.jid.ends_with("@g.us") {
        ""
    } else {
        message.sender_name.trim()
    };
    conn.execute(
        "INSERT INTO wa_conversas (jid, nome, grupo, ultima_em) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(jid) DO UPDATE SET
            ultima_em = CASE WHEN wa_conversas.ultima_em IS NULL OR excluded.ultima_em > wa_conversas.ultima_em
                THEN excluded.ultima_em ELSE wa_conversas.ultima_em END,
            nome = CASE WHEN wa_conversas.nome = '' THEN excluded.nome ELSE wa_conversas.nome END",
        params![
            message.jid,
            fallback_name,
            message.jid.ends_with("@g.us"),
            message.at
        ],
    )?;
    Ok(inserted)
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    let kind: String = row.get(7)?;
    Ok(Message {
        id: row.get(0)?,
        jid: row.get(1)?,
        sender: row.get(2)?,
        sender_name: row.get(3)?,
        text: row.get(4)?,
        mine: row.get(5)?,
        at: row.get(6)?,
        kind: MessageKind::parse(&kind),
    })
}

pub fn messages(conn: &Connection, jid: &str, limit: usize) -> rusqlite::Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT id, jid, de, nome_de, texto, minha, em, tipo FROM (
            SELECT rowid AS ordem, * FROM wa_mensagens WHERE jid = ?1 ORDER BY em DESC, rowid DESC LIMIT ?2
         ) ORDER BY em, ordem",
    )?;
    stmt.query_map(params![jid, limit as i64], message_from_row)?
        .collect()
}

pub fn conversations(conn: &Connection) -> rusqlite::Result<Vec<Conversation>> {
    let mut stmt = conn.prepare(
        "SELECT c.jid, c.nome, a.apelido, c.grupo, c.ultima_em,
            (SELECT CASE WHEN m.texto <> '' THEN m.texto ELSE '[' || m.tipo || ']' END
             FROM wa_mensagens m WHERE m.jid = c.jid ORDER BY m.em DESC LIMIT 1)
         FROM wa_conversas c LEFT JOIN wa_apelidos a ON a.jid = c.jid
         ORDER BY c.ultima_em IS NULL, c.ultima_em DESC, COALESCE(a.apelido, c.nome)",
    )?;
    stmt.query_map([], |row| {
        Ok(Conversation {
            jid: row.get(0)?,
            name: row.get(1)?,
            alias: row.get(2)?,
            group: row.get(3)?,
            last_at: row.get(4)?,
            preview: row.get(5)?,
        })
    })?
    .collect()
}

pub fn set_alias(conn: &Connection, jid: &str, alias: &str) -> rusqlite::Result<()> {
    let alias = alias.trim();
    if alias.is_empty() {
        conn.execute("DELETE FROM wa_apelidos WHERE jid = ?1", [jid])?;
    } else {
        conn.execute(
            "INSERT INTO wa_apelidos (jid, apelido) VALUES (?1, ?2)
             ON CONFLICT(jid) DO UPDATE SET apelido = excluded.apelido",
            params![jid, alias],
        )?;
    }
    Ok(())
}

pub fn find_conversation(conn: &Connection, query: &str) -> rusqlite::Result<Option<Conversation>> {
    let query = query.trim();
    if query.contains('@') {
        let jid: Option<String> = conn
            .query_row(
                "SELECT jid FROM wa_conversas WHERE jid = ?1",
                [query],
                |row| row.get(0),
            )
            .optional()?;
        if jid.is_none() {
            return Ok(None);
        }
    }
    let list = conversations(conn)?;
    let lower = query.to_lowercase();
    Ok(list
        .iter()
        .find(|c| c.jid == query || c.title().to_lowercase() == lower)
        .or_else(|| list.iter().find(|c| c.matches(query)))
        .cloned())
}

pub fn sidecar_command(data_dir: &Path) -> CommandSpec {
    let file_name = format!("{SIDECAR_NAME}{}", std::env::consts::EXE_SUFFIX);
    let beside_exe: Option<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&file_name)))
        .filter(|path| path.is_file());
    let program = beside_exe
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| SIDECAR_NAME.into());
    CommandSpec::new(program)
        .arg("--dados")
        .arg(data_dir.as_os_str().to_os_string())
}

#[derive(Clone, Default)]
pub struct WhatsAppLink(Arc<Mutex<Option<BridgeHandle>>>);

impl WhatsAppLink {
    pub fn set(&self, handle: Option<BridgeHandle>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = handle;
    }

    pub fn get(&self) -> Option<BridgeHandle> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests;
