pub mod convert;
pub mod engine;
pub mod files;
pub mod protocol;
pub mod service;

pub use service::{Status, WhatsAppLink};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension, Row, params};

pub use engine::BridgeHandle;
pub use protocol::{
    WireChatInfo, WireConversation, WireEdit, WireMention, WireMessage, WireParticipant,
    WireReaction, WireRevoke,
};

pub const MESSAGES_LIMIT: usize = 200;
pub const HISTORY_BATCH: u32 = 50;
pub const PHOTO_VALIDITY_DAYS: i64 = 7;
const PENDING_KIND: &str = "pendente";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageKind {
    #[default]
    Text,
    Image,
    Audio,
    Video,
    Document,
    Sticker,
    Contact,
    Location,
    Poll,
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
            "contato" => MessageKind::Contact,
            "localizacao" => MessageKind::Location,
            "enquete" => MessageKind::Poll,
            _ => MessageKind::Other,
        }
    }

    pub fn as_text(self) -> &'static str {
        match self {
            MessageKind::Text => "texto",
            MessageKind::Image => "imagem",
            MessageKind::Audio => "audio",
            MessageKind::Video => "video",
            MessageKind::Document => "documento",
            MessageKind::Sticker => "figurinha",
            MessageKind::Contact => "contato",
            MessageKind::Location => "localizacao",
            MessageKind::Poll => "enquete",
            MessageKind::Other => "outro",
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
            MessageKind::Contact => Some("[contato]"),
            MessageKind::Location => Some("[localização]"),
            MessageKind::Poll => Some("[enquete]"),
            MessageKind::Other => Some("[mensagem não suportada]"),
        }
    }

    pub fn is_media(self) -> bool {
        matches!(
            self,
            MessageKind::Image
                | MessageKind::Audio
                | MessageKind::Video
                | MessageKind::Document
                | MessageKind::Sticker
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    pub id: String,
    pub jid: String,
    pub sender: String,
    pub sender_name: String,
    pub text: String,
    pub mine: bool,
    pub at: NaiveDateTime,
    pub kind: MessageKind,
    pub downloadable: bool,
    pub quoted_id: String,
    pub quoted_from: String,
    pub quoted_name: String,
    pub quoted_text: String,
    pub edited: bool,
    pub deleted: bool,
    pub forwarded: bool,
    pub file: String,
    pub file_name: String,
    pub mime: String,
    pub duration: u32,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub voice: bool,
    pub sender_raw: String,
    pub raw: Vec<u8>,
}

impl Message {
    pub fn sender_label(&self) -> String {
        let name = self.sender_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        phone_label(&self.sender).unwrap_or_else(|| "Participante".to_string())
    }

    pub fn quoted_label(&self) -> String {
        let name = self.quoted_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        phone_label(&self.quoted_from).unwrap_or_else(|| "Participante".to_string())
    }

    pub fn has_media(&self) -> bool {
        self.kind == MessageKind::Image && self.downloadable && !self.deleted
    }

    pub fn display_text(&self) -> String {
        if self.deleted {
            return "[mensagem apagada]".to_string();
        }
        match (self.kind.placeholder(), self.text.trim().is_empty()) {
            (Some(placeholder), true) => placeholder.to_string(),
            (Some(placeholder), false) => format!("{placeholder} {}", self.text),
            (None, _) => self.text.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub message_id: String,
    pub sender: String,
    pub emoji: String,
    pub mine: bool,
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
    pub pinned: bool,
    pub photo: Option<String>,
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

pub fn to_unix_millis(at: NaiveDateTime) -> i64 {
    Local
        .from_local_datetime(&at)
        .earliest()
        .map(|local| local.timestamp_millis())
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
        downloadable: !wire.bruto.is_empty(),
        quoted_id: wire.citada_id.clone(),
        quoted_from: wire.citada_de.clone(),
        quoted_name: String::new(),
        quoted_text: wire.citada_texto.clone(),
        edited: false,
        deleted: false,
        forwarded: wire.encaminhada,
        file: String::new(),
        file_name: wire.nome_arquivo.clone(),
        mime: wire.mime.clone(),
        duration: wire.duracao,
        size: wire.tamanho,
        width: wire.largura,
        height: wire.altura,
        voice: wire.voz,
        sender_raw: wire.autor_bruto.clone(),
        raw: wire.bruto.clone(),
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

pub fn bare_jid(jid: &str) -> String {
    match jid.split_once('@') {
        Some((user, server)) => {
            let user = user.split(':').next().unwrap_or(user);
            format!("{user}@{server}")
        }
        None => jid.to_string(),
    }
}

fn canonical_jid(conn: &Connection, jid: &str) -> String {
    let bare = bare_jid(jid);
    conn.query_row("SELECT pn FROM wa_lids WHERE lid = ?1", [&bare], |row| {
        row.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
    .unwrap_or(bare)
}

fn same_author(
    conn: &Connection,
    stored: &str,
    stored_raw: &str,
    stored_mine: bool,
    author: &str,
    author_raw: &str,
    author_mine: bool,
) -> bool {
    if stored_mine && author_mine {
        return true;
    }
    if stored_mine != author_mine {
        return false;
    }
    let candidate = canonical_jid(conn, author);
    let raw_candidate = (!author_raw.is_empty()).then(|| canonical_jid(conn, author_raw));
    [stored, stored_raw]
        .into_iter()
        .filter(|jid| !jid.is_empty())
        .map(|jid| canonical_jid(conn, jid))
        .any(|jid| jid == candidate || raw_candidate.as_deref() == Some(jid.as_str()))
}

struct Existing {
    kind: String,
    sender: String,
    sender_raw: String,
    mine: bool,
    name: String,
    has_raw: bool,
    text: String,
    quoted_id: String,
    edited: bool,
    deleted: bool,
}

fn existing(conn: &Connection, jid: &str, id: &str) -> rusqlite::Result<Option<Existing>> {
    conn.prepare_cached(
        "SELECT tipo, de, autor_bruto, minha, nome_de, bruto IS NOT NULL, texto, citada_id,
            editada, apagada
         FROM wa_mensagens WHERE jid = ?1 AND id = ?2",
    )?
    .query_row(params![jid, id], |row| {
        Ok(Existing {
            kind: row.get(0)?,
            sender: row.get(1)?,
            sender_raw: row.get(2)?,
            mine: row.get(3)?,
            name: row.get(4)?,
            has_raw: row.get(5)?,
            text: row.get(6)?,
            quoted_id: row.get(7)?,
            edited: row.get(8)?,
            deleted: row.get(9)?,
        })
    })
    .optional()
}

fn raw_param(raw: &[u8]) -> Option<&[u8]> {
    (!raw.is_empty()).then_some(raw)
}

pub fn store_message(conn: &Connection, message: &Message) -> rusqlite::Result<bool> {
    let changed = match existing(conn, &message.jid, &message.id)? {
        None => {
            insert_message(conn, message)?;
            true
        }
        Some(found) if found.kind == PENDING_KIND => {
            fulfil_pending(conn, message, &found)?;
            true
        }
        Some(found) => refresh_message(conn, message, &found)?,
    };
    let fallback_name = if message.mine || message.jid.ends_with("@g.us") {
        ""
    } else {
        message.sender_name.trim()
    };
    conn.prepare_cached(
        "INSERT INTO wa_conversas (jid, nome, grupo, ultima_em) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(jid) DO UPDATE SET
            ultima_em = CASE WHEN wa_conversas.ultima_em IS NULL OR excluded.ultima_em > wa_conversas.ultima_em
                THEN excluded.ultima_em ELSE wa_conversas.ultima_em END,
            nome = CASE WHEN wa_conversas.nome = '' THEN excluded.nome ELSE wa_conversas.nome END",
    )?
    .execute(
        params![
            message.jid,
            fallback_name,
            message.jid.ends_with("@g.us"),
            message.at
        ],
    )?;
    Ok(changed)
}

fn insert_message(conn: &Connection, m: &Message) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO wa_mensagens (id, jid, de, nome_de, texto, minha, em, tipo, midia, bruto,
            citada_id, citada_de, citada_texto, editada, apagada, encaminhada, arquivo,
            nome_arquivo, mime, duracao, tamanho, largura, altura, voz, autor_bruto)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '', ?9, ?10, ?11, ?12, 0, 0, ?13, ?14, ?15, ?16,
            ?17, ?18, ?19, ?20, ?21, ?22)",
    )?
    .execute(params![
        m.id,
        m.jid,
        m.sender,
        m.sender_name,
        m.text,
        m.mine,
        m.at,
        m.kind.as_text(),
        raw_param(&m.raw),
        m.quoted_id,
        m.quoted_from,
        m.quoted_text,
        m.forwarded,
        m.file,
        m.file_name,
        m.mime,
        m.duration,
        m.size as i64,
        m.width,
        m.height,
        m.voice,
        m.sender_raw,
    ])?;
    Ok(())
}

fn fulfil_pending(conn: &Connection, m: &Message, found: &Existing) -> rusqlite::Result<()> {
    let valid = found.sender.is_empty()
        || same_author(
            conn,
            &found.sender,
            &found.sender_raw,
            found.mine,
            &m.sender,
            &m.sender_raw,
            m.mine,
        );
    let edited = valid && found.edited;
    let deleted = valid && found.deleted;
    let text = if deleted {
        String::new()
    } else if edited {
        found.text.clone()
    } else {
        m.text.clone()
    };
    let raw = if deleted { None } else { raw_param(&m.raw) };
    conn.execute(
        "UPDATE wa_mensagens SET de = ?3, nome_de = ?4, texto = ?5, minha = ?6, em = ?7, tipo = ?8,
            bruto = ?9, citada_id = ?10, citada_de = ?11, citada_texto = ?12, editada = ?13,
            apagada = ?14, encaminhada = ?15, nome_arquivo = ?16, mime = ?17, duracao = ?18,
            tamanho = ?19, largura = ?20, altura = ?21, voz = ?22, autor_bruto = ?23
         WHERE jid = ?1 AND id = ?2",
        params![
            m.jid,
            m.id,
            m.sender,
            m.sender_name,
            text,
            m.mine,
            m.at,
            m.kind.as_text(),
            raw,
            m.quoted_id,
            m.quoted_from,
            m.quoted_text,
            edited,
            deleted,
            m.forwarded,
            m.file_name,
            m.mime,
            m.duration,
            m.size as i64,
            m.width,
            m.height,
            m.voice,
            m.sender_raw,
        ],
    )?;
    Ok(())
}

fn refresh_message(conn: &Connection, m: &Message, found: &Existing) -> rusqlite::Result<bool> {
    let mut changed = false;
    let mut update = |column: &str, value: &dyn ToSql| -> rusqlite::Result<()> {
        conn.execute(
            &format!("UPDATE wa_mensagens SET {column} = ?3 WHERE jid = ?1 AND id = ?2"),
            params![m.jid, m.id, value],
        )?;
        changed = true;
        Ok(())
    };
    if !m.sender_name.is_empty() && m.sender_name != found.name {
        update("nome_de", &m.sender_name)?;
    }
    if !found.has_raw && !m.raw.is_empty() && !found.deleted {
        update("bruto", &m.raw)?;
    }
    if found.sender_raw.is_empty() && !m.sender_raw.is_empty() {
        update("autor_bruto", &m.sender_raw)?;
    }
    if found.quoted_id.is_empty() && !m.quoted_id.is_empty() {
        update("citada_id", &m.quoted_id)?;
        update("citada_de", &m.quoted_from)?;
        update("citada_texto", &m.quoted_text)?;
    }
    if found.text.is_empty() && !m.text.is_empty() && !found.edited && !found.deleted {
        update("texto", &m.text)?;
    }
    Ok(changed)
}

pub fn store_messages(conn: &Connection, messages: &[Message]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    for message in messages {
        store_message(&tx, message)?;
    }
    tx.commit()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Done,
    Pending,
    Rejected,
}

pub fn apply_edit(conn: &Connection, edit: &WireEdit) -> rusqlite::Result<Applied> {
    let at = from_unix(edit.em);
    match existing(conn, &edit.jid, &edit.id)? {
        None => {
            conn.execute(
                "INSERT INTO wa_mensagens (id, jid, de, minha, texto, em, tipo, editada, autor_bruto)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?3)",
                params![edit.id, edit.jid, edit.de, edit.minha, edit.texto, at, PENDING_KIND],
            )?;
            Ok(Applied::Pending)
        }
        Some(found) if found.kind == PENDING_KIND => {
            conn.execute(
                "UPDATE wa_mensagens SET texto = ?3, editada = 1
                 WHERE jid = ?1 AND id = ?2 AND autor_bruto = ?4",
                params![edit.jid, edit.id, edit.texto, edit.de],
            )?;
            Ok(Applied::Pending)
        }
        Some(found) => {
            if !same_author(
                conn,
                &found.sender,
                &found.sender_raw,
                found.mine,
                &edit.de,
                &edit.de,
                edit.minha,
            ) || found.deleted
            {
                return Ok(Applied::Rejected);
            }
            conn.execute(
                "UPDATE wa_mensagens SET texto = ?3, editada = 1 WHERE jid = ?1 AND id = ?2",
                params![edit.jid, edit.id, edit.texto],
            )?;
            Ok(Applied::Done)
        }
    }
}

pub fn apply_revoke(conn: &Connection, revoke: &WireRevoke) -> rusqlite::Result<Applied> {
    let at = from_unix(revoke.em);
    let by_admin = revoke.admin && is_group_jid(&revoke.jid);
    match existing(conn, &revoke.jid, &revoke.id)? {
        None => {
            let author = if by_admin { "" } else { revoke.de.as_str() };
            conn.execute(
                "INSERT INTO wa_mensagens (id, jid, de, minha, texto, em, tipo, apagada, autor_bruto)
                 VALUES (?1, ?2, ?3, ?4, '', ?5, ?6, 1, ?3)",
                params![revoke.id, revoke.jid, author, revoke.minha, at, PENDING_KIND],
            )?;
            Ok(Applied::Pending)
        }
        Some(found) if found.kind == PENDING_KIND => {
            conn.execute(
                "UPDATE wa_mensagens SET apagada = 1, texto = ''
                 WHERE jid = ?1 AND id = ?2 AND (?3 OR autor_bruto = ?4)",
                params![revoke.jid, revoke.id, by_admin, revoke.de],
            )?;
            Ok(Applied::Pending)
        }
        Some(found) => {
            let allowed = by_admin
                || same_author(
                    conn,
                    &found.sender,
                    &found.sender_raw,
                    found.mine,
                    &revoke.de,
                    &revoke.de,
                    revoke.minha,
                );
            if !allowed {
                return Ok(Applied::Rejected);
            }
            conn.execute(
                "UPDATE wa_mensagens SET apagada = 1, texto = '', bruto = NULL, citada_texto = ''
                 WHERE jid = ?1 AND id = ?2",
                params![revoke.jid, revoke.id],
            )?;
            Ok(Applied::Done)
        }
    }
}

pub fn apply_reaction(conn: &Connection, reaction: &WireReaction) -> rusqlite::Result<()> {
    if reaction.emoji.trim().is_empty() {
        conn.execute(
            "DELETE FROM wa_reacoes WHERE jid = ?1 AND msg_id = ?2 AND de = ?3",
            params![reaction.jid, reaction.id, reaction.de],
        )?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO wa_reacoes (jid, msg_id, de, emoji, minha) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(jid, msg_id, de) DO UPDATE SET emoji = excluded.emoji, minha = excluded.minha",
        params![
            reaction.jid,
            reaction.id,
            reaction.de,
            reaction.emoji,
            reaction.minha
        ],
    )?;
    Ok(())
}

pub fn reactions(conn: &Connection, jid: &str) -> rusqlite::Result<Vec<Reaction>> {
    let mut stmt = conn.prepare(
        "SELECT r.msg_id, COALESCE(l.pn, r.de), r.emoji, r.minha
         FROM wa_reacoes r LEFT JOIN wa_lids l ON l.lid = r.de
         WHERE r.jid = ?1 ORDER BY r.rowid",
    )?;
    stmt.query_map([jid], |row| {
        Ok(Reaction {
            message_id: row.get(0)?,
            sender: row.get(1)?,
            emoji: row.get(2)?,
            mine: row.get(3)?,
        })
    })?
    .collect()
}

pub fn delete_local(conn: &Connection, jid: &str, id: &str) -> rusqlite::Result<bool> {
    conn.execute(
        "DELETE FROM wa_reacoes WHERE jid = ?1 AND msg_id = ?2",
        params![jid, id],
    )?;
    Ok(conn.execute(
        "DELETE FROM wa_mensagens WHERE jid = ?1 AND id = ?2",
        params![jid, id],
    )? > 0)
}

pub fn set_file(conn: &Connection, jid: &str, id: &str, path: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE wa_mensagens SET arquivo = ?3 WHERE jid = ?1 AND id = ?2",
        params![jid, id, path],
    )?;
    Ok(())
}

pub fn photo(
    conn: &Connection,
    jid: &str,
    now: NaiveDateTime,
) -> rusqlite::Result<Option<Option<String>>> {
    let found: Option<(String, NaiveDateTime)> = conn
        .query_row(
            "SELECT caminho, atualizada_em FROM wa_fotos WHERE jid = ?1",
            [jid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(found.and_then(|(path, at)| {
        (now - at < chrono::TimeDelta::days(PHOTO_VALIDITY_DAYS))
            .then(|| (!path.is_empty()).then_some(path))
    }))
}

pub fn set_photo(
    conn: &Connection,
    jid: &str,
    path: &str,
    at: NaiveDateTime,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO wa_fotos (jid, caminho, atualizada_em) VALUES (?1, ?2, ?3)
         ON CONFLICT(jid) DO UPDATE SET caminho = excluded.caminho, atualizada_em = excluded.atualizada_em",
        params![jid, path, at],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRaw {
    pub raw: Vec<u8>,
    pub legacy_media: String,
    pub sender: String,
    pub sender_raw: String,
    pub mine: bool,
    pub kind: MessageKind,
    pub file: String,
    pub file_name: String,
    pub mime: String,
}

pub fn raw_message(conn: &Connection, jid: &str, id: &str) -> rusqlite::Result<Option<StoredRaw>> {
    conn.query_row(
        "SELECT COALESCE(bruto, X''), midia, de, autor_bruto, minha, tipo, arquivo, nome_arquivo, mime
         FROM wa_mensagens WHERE jid = ?1 AND id = ?2 AND tipo <> 'pendente'",
        params![jid, id],
        |row| {
            let kind: String = row.get(5)?;
            Ok(StoredRaw {
                raw: row.get(0)?,
                legacy_media: row.get(1)?,
                sender: row.get(2)?,
                sender_raw: row.get(3)?,
                mine: row.get(4)?,
                kind: MessageKind::parse(&kind),
                file: row.get(6)?,
                file_name: row.get(7)?,
                mime: row.get(8)?,
            })
        },
    )
    .optional()
}

pub fn oldest_message(
    conn: &Connection,
    jid: &str,
) -> rusqlite::Result<Option<(String, bool, NaiveDateTime)>> {
    conn.query_row(
        "SELECT id, minha, em FROM wa_mensagens WHERE jid = ?1 AND tipo <> 'pendente'
         ORDER BY em, rowid LIMIT 1",
        [jid],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
}

pub fn display_name(conn: &Connection, jid: &str) -> Option<String> {
    let alias: Option<String> = conn
        .query_row(
            "SELECT apelido FROM wa_apelidos WHERE jid = ?1",
            [jid],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if alias.is_some() {
        return alias;
    }
    conn.query_row(
        "SELECT nome FROM wa_conversas WHERE jid = ?1 AND grupo = 0 AND nome <> ''",
        [jid],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

pub(crate) fn replace_mention(text: &str, token: &str, name: &str) -> String {
    let needle = format!("@{token}");
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in text.match_indices(&needle) {
        let end = start + needle.len();
        let boundary = text[end..]
            .chars()
            .next()
            .is_none_or(|next| !next.is_alphanumeric());
        if start >= last && boundary {
            out.push_str(&text[last..start]);
            out.push('@');
            out.push_str(name);
            last = end;
        }
    }
    out.push_str(&text[last..]);
    out
}

pub fn resolve_mention_names(conn: &Connection, text: &str, mentions: &[WireMention]) -> String {
    let mut ordered: Vec<&WireMention> = mentions.iter().collect();
    ordered.sort_by_key(|mention| std::cmp::Reverse(mention.token.len()));
    let mut out = text.to_string();
    for mention in ordered {
        let name = display_name(conn, &mention.jid)
            .or_else(|| (!mention.name.trim().is_empty()).then(|| mention.name.trim().to_string()));
        if let Some(name) = name {
            out = replace_mention(&out, &mention.token, &name);
        }
    }
    out
}

const MESSAGE_LIST: &str = "SELECT m.id, m.jid, COALESCE(l.pn, m.de),
        COALESCE(a.apelido, NULLIF(c.nome, ''), m.nome_de),
        m.texto, m.minha, m.em, m.tipo, (m.bruto IS NOT NULL OR m.midia <> ''),
        m.citada_id, COALESCE(ql.pn, m.citada_de),
        COALESCE(qa.apelido, NULLIF(qc.nome, ''), ''), m.citada_texto,
        m.editada, m.apagada, m.encaminhada, m.arquivo, m.nome_arquivo, m.mime,
        m.duracao, m.tamanho, m.largura, m.altura, m.voz, m.autor_bruto
     FROM (%INNER%) m
     LEFT JOIN wa_lids l ON l.lid = m.de
     LEFT JOIN wa_apelidos a ON a.jid = COALESCE(l.pn, m.de)
     LEFT JOIN wa_conversas c ON c.jid = COALESCE(l.pn, m.de) AND c.grupo = 0
     LEFT JOIN wa_lids ql ON ql.lid = m.citada_de
     LEFT JOIN wa_apelidos qa ON qa.jid = COALESCE(ql.pn, m.citada_de)
     LEFT JOIN wa_conversas qc ON qc.jid = COALESCE(ql.pn, m.citada_de) AND qc.grupo = 0";

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
        downloadable: row.get(8)?,
        quoted_id: row.get(9)?,
        quoted_from: row.get(10)?,
        quoted_name: row.get(11)?,
        quoted_text: row.get(12)?,
        edited: row.get(13)?,
        deleted: row.get(14)?,
        forwarded: row.get(15)?,
        file: row.get(16)?,
        file_name: row.get(17)?,
        mime: row.get(18)?,
        duration: row.get(19)?,
        size: row.get::<_, i64>(20)?.max(0) as u64,
        width: row.get(21)?,
        height: row.get(22)?,
        voice: row.get(23)?,
        sender_raw: row.get(24)?,
        raw: Vec::new(),
    })
}

fn select_messages(
    conn: &Connection,
    condition: &str,
    args: &[&dyn ToSql],
    newest_first: bool,
    limit: usize,
) -> rusqlite::Result<Vec<Message>> {
    let limit = limit as i64;
    let mut all: Vec<&dyn ToSql> = args.to_vec();
    all.push(&limit);
    let inner = format!(
        "SELECT rowid AS ordem, * FROM wa_mensagens WHERE tipo <> 'pendente' AND ({condition})
         ORDER BY em DESC, rowid DESC LIMIT ?{}",
        all.len()
    );
    let order = if newest_first {
        "ORDER BY m.em DESC, m.ordem DESC"
    } else {
        "ORDER BY m.em, m.ordem"
    };
    let sql = format!("{} {order}", MESSAGE_LIST.replace("%INNER%", &inner));
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_map(rusqlite::params_from_iter(all), message_from_row)?
        .collect()
}

pub fn messages(conn: &Connection, jid: &str, limit: usize) -> rusqlite::Result<Vec<Message>> {
    select_messages(conn, "jid = ?1", &[&jid], false, limit)
}

pub fn messages_before(
    conn: &Connection,
    jid: &str,
    at: NaiveDateTime,
    id: &str,
    limit: usize,
) -> rusqlite::Result<Vec<Message>> {
    select_messages(
        conn,
        "jid = ?1 AND (em < ?2 OR (em = ?2 AND rowid < COALESCE(
            (SELECT rowid FROM wa_mensagens WHERE jid = ?1 AND id = ?3), 9223372036854775807)))",
        &[&jid, &at, &id],
        false,
        limit,
    )
}

fn escape_like(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StickerSection {
    #[default]
    Recent,
    Favorite,
    Received,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sticker {
    pub hash: String,
    pub action: Vec<u8>,
    pub file: String,
}

const STICKER_SCAN: usize = 400;

pub fn stickers(
    conn: &Connection,
    section: StickerSection,
    limit: usize,
) -> rusqlite::Result<Vec<Sticker>> {
    if section == StickerSection::Favorite {
        let mut stmt =
            conn.prepare("SELECT hash, acao FROM wa_figurinhas ORDER BY em DESC LIMIT ?1")?;
        return stmt
            .query_map([limit as i64], |row| {
                Ok(Sticker {
                    hash: row.get(0)?,
                    action: row.get(1)?,
                    file: String::new(),
                })
            })?
            .collect();
    }
    let mut stmt = conn.prepare(
        "SELECT bruto, arquivo FROM wa_mensagens
         WHERE tipo = 'figurinha' AND apagada = 0 AND bruto IS NOT NULL AND minha = ?1
         ORDER BY em DESC LIMIT ?2",
    )?;
    let rows: Vec<(Vec<u8>, String)> = stmt
        .query_map(
            params![section == StickerSection::Recent, STICKER_SCAN as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
        .collect::<rusqlite::Result<_>>()?;
    let mut seen = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        .filter_map(|(raw, file)| {
            let (hash, action) = engine::sticker_action(&raw)?;
            seen.insert(hash.clone())
                .then_some(Sticker { hash, action, file })
        })
        .take(limit)
        .collect())
}

pub fn favorite_hashes(conn: &Connection) -> rusqlite::Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare("SELECT hash FROM wa_figurinhas")?;
    stmt.query_map([], |row| row.get(0))?.collect()
}

pub fn set_favorite_sticker(
    conn: &Connection,
    hash: &str,
    action: &[u8],
    favorite: bool,
    at: i64,
) -> rusqlite::Result<()> {
    if favorite {
        conn.execute(
            "INSERT INTO wa_figurinhas (hash, acao, em) VALUES (?1, ?2, ?3)
             ON CONFLICT(hash) DO UPDATE SET acao = excluded.acao, em = excluded.em",
            params![hash, action, at],
        )?;
    } else {
        conn.execute("DELETE FROM wa_figurinhas WHERE hash = ?1", [hash])?;
    }
    Ok(())
}

pub fn search_messages(
    conn: &Connection,
    query: &str,
    jid: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Message>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = format!("%{}%", escape_like(query));
    let scope = jid.unwrap_or_default();
    select_messages(
        conn,
        "apagada = 0 AND (texto LIKE ?1 ESCAPE '\\' OR nome_arquivo LIKE ?1 ESCAPE '\\')
         AND (?2 = '' OR jid = ?2)",
        &[&pattern, &scope],
        true,
        limit,
    )
}

const CONVERSATION_COLUMNS: &str =
    "SELECT c.jid, c.nome, a.apelido, c.grupo, c.arquivada, c.ultima_em,
        (SELECT CASE WHEN m.apagada THEN '[mensagem apagada]'
                     WHEN m.texto <> '' THEN m.texto ELSE '[' || m.tipo || ']' END
         FROM wa_mensagens m WHERE m.jid = c.jid AND m.tipo <> 'pendente'
         ORDER BY m.em DESC LIMIT 1),
        c.notificar, c.fixada IS NOT NULL,
        (SELECT f.caminho FROM wa_fotos f WHERE f.jid = c.jid AND f.caminho <> '')
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
        pinned: row.get(8)?,
        photo: row.get(9)?,
    })
}

pub fn conversations(conn: &Connection) -> rusqlite::Result<Vec<Conversation>> {
    let mut stmt = conn.prepare(&format!(
        "{CONVERSATION_COLUMNS}
         ORDER BY c.fixada IS NULL, c.fixada, c.ultima_em IS NULL, c.ultima_em DESC,
             COALESCE(a.apelido, c.nome)"
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

pub fn set_pinned_order(conn: &Connection, jids: &[String]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE wa_conversas SET fixada = NULL WHERE fixada IS NOT NULL",
        [],
    )?;
    for (position, jid) in jids.iter().enumerate() {
        tx.execute(
            "UPDATE wa_conversas SET fixada = ?2 WHERE jid = ?1",
            params![jid, position as i64],
        )?;
    }
    tx.commit()
}

pub fn pinned_jids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn
        .prepare_cached("SELECT jid FROM wa_conversas WHERE fixada IS NOT NULL ORDER BY fixada")?;
    stmt.query_map([], |row| row.get(0))?.collect()
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
        if let Some(archived) = info.arquivada {
            tx.execute(
                "UPDATE wa_conversas SET arquivada = ?2 WHERE jid = ?1",
                params![target, archived],
            )?;
        }
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
        "UPDATE OR IGNORE wa_reacoes SET jid = ?2 WHERE jid = ?1",
        params![lid, pn],
    )?;
    conn.execute("DELETE FROM wa_reacoes WHERE jid = ?1", [lid])?;
    conn.execute(
        "INSERT OR IGNORE INTO wa_fotos (jid, caminho, atualizada_em)
         SELECT ?2, caminho, atualizada_em FROM wa_fotos WHERE jid = ?1",
        params![lid, pn],
    )?;
    conn.execute("DELETE FROM wa_fotos WHERE jid = ?1", [lid])?;
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

#[cfg(test)]
mod tests;
