pub mod bridge;
pub mod protocol;
pub mod service;

pub use service::{Status, WhatsAppLink};

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::platform::CommandSpec;
pub use bridge::BridgeHandle;
pub use protocol::{WireChatInfo, WireConversation, WireMessage};

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
    pub media: String,
}

impl Message {
    pub fn sender_label(&self) -> String {
        let name = self.sender_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        phone_label(&self.sender).unwrap_or_else(|| "Participante".to_string())
    }

    pub fn has_media(&self) -> bool {
        self.kind == MessageKind::Image && !self.media.is_empty()
    }

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
    pub archived: bool,
    pub last_at: Option<NaiveDateTime>,
    pub preview: Option<String>,
    pub notify: bool,
}

impl Conversation {
    pub fn title(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        if !self.name.trim().is_empty() {
            return self.name.clone();
        }
        if self.group {
            return "Grupo sem nome".to_string();
        }
        phone_label(&self.jid).unwrap_or_else(|| "Contato sem nome".to_string())
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.title().to_lowercase().contains(&query)
            || self.name.to_lowercase().contains(&query)
            || self.jid.contains(&query)
    }
}

pub fn phone_label(jid: &str) -> Option<String> {
    let number = jid.strip_suffix("@s.whatsapp.net")?;
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let formatted = match (number.strip_prefix("55"), number.len()) {
        (Some(rest), 12 | 13) => {
            let (area, local) = rest.split_at(2);
            let (head, tail) = local.split_at(local.len() - 4);
            format!("+55 {area} {head}-{tail}")
        }
        _ => format!("+{number}"),
    };
    Some(formatted)
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
        media: wire.midia.clone(),
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
        "INSERT INTO wa_mensagens (id, jid, de, nome_de, texto, minha, em, tipo, midia)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(jid, id) DO UPDATE SET
            midia = CASE WHEN excluded.midia <> '' THEN excluded.midia ELSE wa_mensagens.midia END,
            nome_de = CASE WHEN excluded.nome_de <> '' THEN excluded.nome_de ELSE wa_mensagens.nome_de END
         WHERE excluded.midia <> wa_mensagens.midia OR excluded.nome_de <> wa_mensagens.nome_de",
        params![
            message.id,
            message.jid,
            message.sender,
            message.sender_name,
            message.text,
            message.mine,
            message.at,
            kind_text(message.kind),
            message.media
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

pub fn store_messages(conn: &Connection, messages: &[Message]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for message in messages {
        store_message(&tx, message)?;
    }
    tx.commit()
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
        media: row.get(8)?,
    })
}

pub fn messages(conn: &Connection, jid: &str, limit: usize) -> rusqlite::Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.jid, COALESCE(l.pn, m.de),
            COALESCE(a.apelido, NULLIF(c.nome, ''), m.nome_de),
            m.texto, m.minha, m.em, m.tipo, m.midia
         FROM (
            SELECT rowid AS ordem, * FROM wa_mensagens WHERE jid = ?1 ORDER BY em DESC, rowid DESC LIMIT ?2
         ) m
         LEFT JOIN wa_lids l ON l.lid = m.de
         LEFT JOIN wa_apelidos a ON a.jid = COALESCE(l.pn, m.de)
         LEFT JOIN wa_conversas c ON c.jid = COALESCE(l.pn, m.de) AND c.grupo = 0
         ORDER BY m.em, m.ordem",
    )?;
    stmt.query_map(params![jid, limit as i64], message_from_row)?
        .collect()
}

const CONVERSATION_COLUMNS: &str =
    "SELECT c.jid, c.nome, a.apelido, c.grupo, c.arquivada, c.ultima_em,
        (SELECT CASE WHEN m.texto <> '' THEN m.texto ELSE '[' || m.tipo || ']' END
         FROM wa_mensagens m WHERE m.jid = c.jid ORDER BY m.em DESC LIMIT 1),
        c.notificar
     FROM wa_conversas c LEFT JOIN wa_apelidos a ON a.jid = c.jid";

fn conversation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        jid: row.get(0)?,
        name: row.get(1)?,
        alias: row.get(2)?,
        group: row.get(3)?,
        archived: row.get(4)?,
        last_at: row.get(5)?,
        preview: row.get(6)?,
        notify: row.get(7)?,
    })
}

pub fn conversations(conn: &Connection) -> rusqlite::Result<Vec<Conversation>> {
    let mut stmt = conn.prepare(&format!(
        "{CONVERSATION_COLUMNS}
         ORDER BY c.ultima_em IS NULL, c.ultima_em DESC, COALESCE(a.apelido, c.nome)"
    ))?;
    stmt.query_map([], conversation_from_row)?.collect()
}

pub fn conversation(conn: &Connection, jid: &str) -> rusqlite::Result<Option<Conversation>> {
    conn.query_row(
        &format!("{CONVERSATION_COLUMNS} WHERE c.jid = ?1"),
        [jid],
        conversation_from_row,
    )
    .optional()
}

pub fn set_notify(conn: &Connection, jid: &str, notify: bool) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE wa_conversas SET notificar = ?2 WHERE jid = ?1",
        params![jid, notify],
    )?;
    Ok(())
}

pub fn is_group_jid(jid: &str) -> bool {
    jid.ends_with("@g.us")
}

pub fn is_private_jid(jid: &str) -> bool {
    jid.ends_with("@s.whatsapp.net") || jid.ends_with("@lid")
}

pub fn known_jids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT jid FROM wa_conversas WHERE ultima_em IS NOT NULL OR jid LIKE '%@lid'
         UNION
         SELECT DISTINCT de FROM wa_mensagens WHERE de LIKE '%@lid'",
    )?;
    stmt.query_map([], |row| row.get(0))?.collect()
}

pub fn set_archived(conn: &Connection, jid: &str, archived: bool) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE wa_conversas SET arquivada = ?2
         WHERE jid = ?1 OR jid IN (SELECT lid FROM wa_lids WHERE pn = ?1)",
        params![jid, archived],
    )?;
    Ok(())
}

pub fn apply_chat_info(conn: &Connection, infos: &[WireChatInfo]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for info in infos {
        let phone = info.phone.trim();
        let target = if phone.is_empty() || phone == info.jid {
            info.jid.as_str()
        } else {
            tx.execute(
                "INSERT INTO wa_lids (lid, pn) VALUES (?1, ?2)
                 ON CONFLICT(lid) DO UPDATE SET pn = excluded.pn",
                params![info.jid, phone],
            )?;
            merge_conversation(&tx, &info.jid, phone)?;
            phone
        };
        if !info.nome.trim().is_empty() && !target.ends_with("@g.us") {
            tx.execute(
                "INSERT INTO wa_conversas (jid, nome, grupo) VALUES (?1, ?2, 0)
                 ON CONFLICT(jid) DO UPDATE SET nome = excluded.nome WHERE wa_conversas.grupo = 0",
                params![target, info.nome.trim()],
            )?;
        }
        tx.execute(
            "UPDATE wa_conversas SET arquivada = ?2 WHERE jid = ?1",
            params![target, info.arquivada],
        )?;
    }
    tx.commit()
}

fn merge_conversation(conn: &Connection, lid: &str, pn: &str) -> rusqlite::Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wa_conversas WHERE jid = ?1)",
        [lid],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO wa_conversas (jid, nome, grupo, ultima_em, arquivada)
         SELECT ?2, nome, 0, ultima_em, arquivada FROM wa_conversas WHERE jid = ?1
         ON CONFLICT(jid) DO UPDATE SET
            ultima_em = CASE WHEN wa_conversas.ultima_em IS NULL OR excluded.ultima_em > wa_conversas.ultima_em
                THEN excluded.ultima_em ELSE wa_conversas.ultima_em END,
            nome = CASE WHEN wa_conversas.nome = '' THEN excluded.nome ELSE wa_conversas.nome END",
        params![lid, pn],
    )?;
    conn.execute(
        "UPDATE OR IGNORE wa_mensagens SET jid = ?2 WHERE jid = ?1",
        params![lid, pn],
    )?;
    conn.execute("DELETE FROM wa_mensagens WHERE jid = ?1", [lid])?;
    conn.execute(
        "INSERT OR IGNORE INTO wa_apelidos (jid, apelido) SELECT ?2, apelido FROM wa_apelidos WHERE jid = ?1",
        params![lid, pn],
    )?;
    conn.execute("DELETE FROM wa_apelidos WHERE jid = ?1", [lid])?;
    conn.execute("DELETE FROM wa_conversas WHERE jid = ?1", [lid])?;
    Ok(())
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

#[cfg(test)]
mod tests;
