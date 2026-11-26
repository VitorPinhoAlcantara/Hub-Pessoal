use whatsapp_rust::prelude::{MessageExt, MessageField, wa};
use whatsapp_rust::waproto;

use super::MessageKind;
use super::protocol::{WireEdit, WireMention, WireMessage, WireReaction, WireRevoke};

pub const QUOTE_LIMIT: usize = 300;

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub id: String,
    pub chat: String,
    pub sender: String,
    pub sender_raw: String,
    pub from_me: bool,
    pub push_name: String,
    pub ts: i64,
    pub history: bool,
    pub quoted_from: String,
    pub mentions: Vec<WireMention>,
    pub own_tokens: Vec<String>,
    pub admin_revoke: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Message(Box<WireMessage>),
    Edit(WireEdit),
    Revoke(WireRevoke),
    Reaction(WireReaction),
    Skip,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    pub quoted_id: String,
    pub quoted_participant: String,
    pub quoted_text: String,
    pub forwarded: bool,
    pub mentioned: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    pub kind: MessageKind,
    pub text: String,
    pub mime: String,
    pub file_name: String,
    pub duration: u32,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub voice: bool,
}

impl Parts {
    fn new(kind: MessageKind, text: String) -> Self {
        Self {
            kind,
            text,
            mime: String::new(),
            file_name: String::new(),
            duration: 0,
            size: 0,
            width: 0,
            height: 0,
            voice: false,
        }
    }
}

fn is_noise(base: &wa::Message) -> bool {
    let mut rest = base.clone();
    rest.sender_key_distribution_message = MessageField::none();
    rest.fast_ratchet_key_sender_key_distribution_message = MessageField::none();
    rest.message_context_info = MessageField::none();
    rest.protocol_message = MessageField::none();
    rest.poll_update_message = MessageField::none();
    rest.pin_in_chat_message = MessageField::none();
    rest.keep_in_chat_message = MessageField::none();
    rest.call = MessageField::none();
    rest.call_log_messsage = MessageField::none();
    rest.enc_reaction_message = MessageField::none();
    rest.enc_comment_message = MessageField::none();
    rest.placeholder_message = MessageField::none();
    rest.secret_encrypted_message = MessageField::none();
    rest.message_history_bundle = MessageField::none();
    rest.message_history_notice = MessageField::none();
    rest.sticker_sync_rmr_message = MessageField::none();
    rest.status_notification_message = MessageField::none();
    rest.limit_sharing_message = MessageField::none();
    rest.group_root_key_share = MessageField::none();
    rest.root_secret_distribute_message = MessageField::none();
    rest.request_phone_number_message = MessageField::none();
    waproto::codec::message_to_vec(&rest).is_empty()
}

fn poll_text(poll: &wa::message::PollCreationMessage) -> String {
    let mut text = poll.name.clone().unwrap_or_default();
    for (index, option) in poll.options.iter().enumerate() {
        text.push_str(&format!(
            "\n{}. {}",
            index + 1,
            option.option_name.as_deref().unwrap_or_default()
        ));
    }
    text
}

fn location_text(
    name: Option<&str>,
    address: Option<&str>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> String {
    let name = name.unwrap_or("").trim();
    let address = address.unwrap_or("").trim();
    match (name.is_empty(), address.is_empty()) {
        (false, false) => format!("{name}, {address}"),
        (false, true) => name.to_string(),
        (true, false) => address.to_string(),
        (true, true) => match (latitude, longitude) {
            (Some(lat), Some(lon)) => format!("{lat:.6}, {lon:.6}"),
            _ => String::new(),
        },
    }
}

pub fn classify(base: &wa::Message) -> Option<Parts> {
    if let Some(text) = base.conversation.as_deref() {
        return Some(Parts::new(MessageKind::Text, text.to_string()));
    }
    if let Some(ext) = base.extended_text_message.as_option() {
        let text = ext.text.clone().unwrap_or_default();
        return Some(Parts::new(MessageKind::Text, text));
    }
    if let Some(image) = base.image_message.as_option() {
        let mut parts = Parts::new(
            MessageKind::Image,
            image.caption.clone().unwrap_or_default(),
        );
        parts.mime = image.mimetype.clone().unwrap_or_default();
        parts.size = image.file_length.unwrap_or(0);
        parts.width = image.width.unwrap_or(0);
        parts.height = image.height.unwrap_or(0);
        return Some(parts);
    }
    if let Some(video) = base
        .video_message
        .as_option()
        .or_else(|| base.ptv_message.as_option())
    {
        let mut parts = Parts::new(
            MessageKind::Video,
            video.caption.clone().unwrap_or_default(),
        );
        parts.mime = video.mimetype.clone().unwrap_or_default();
        parts.size = video.file_length.unwrap_or(0);
        parts.width = video.width.unwrap_or(0);
        parts.height = video.height.unwrap_or(0);
        parts.duration = video.seconds.unwrap_or(0);
        return Some(parts);
    }
    if let Some(audio) = base.audio_message.as_option() {
        let mut parts = Parts::new(MessageKind::Audio, String::new());
        parts.mime = audio.mimetype.clone().unwrap_or_default();
        parts.size = audio.file_length.unwrap_or(0);
        parts.duration = audio.seconds.unwrap_or(0);
        parts.voice = audio.ptt.unwrap_or(false);
        return Some(parts);
    }
    if let Some(document) = base.document_message.as_option() {
        let name = document.file_name.clone().unwrap_or_default();
        let caption = document.caption.clone().unwrap_or_default();
        let text = if caption.is_empty() {
            name.clone()
        } else {
            caption
        };
        let mut parts = Parts::new(MessageKind::Document, text);
        parts.mime = document.mimetype.clone().unwrap_or_default();
        parts.size = document.file_length.unwrap_or(0);
        parts.file_name = name;
        return Some(parts);
    }
    if let Some(sticker) = base.sticker_message.as_option() {
        let mut parts = Parts::new(MessageKind::Sticker, String::new());
        parts.mime = sticker.mimetype.clone().unwrap_or_default();
        parts.size = sticker.file_length.unwrap_or(0);
        parts.width = sticker.width.unwrap_or(0);
        parts.height = sticker.height.unwrap_or(0);
        return Some(parts);
    }
    if let Some(contact) = base.contact_message.as_option() {
        let name = contact.display_name.clone().unwrap_or_default();
        return Some(Parts::new(MessageKind::Contact, name));
    }
    if let Some(contacts) = base.contacts_array_message.as_option() {
        let name = contacts.display_name.clone().unwrap_or_default();
        let text = if name.is_empty() {
            format!("{} contatos", contacts.contacts.len())
        } else {
            name
        };
        return Some(Parts::new(MessageKind::Contact, text));
    }
    if let Some(location) = base.location_message.as_option() {
        let text = location_text(
            location.name.as_deref(),
            location.address.as_deref(),
            location.degrees_latitude,
            location.degrees_longitude,
        );
        return Some(Parts::new(MessageKind::Location, text));
    }
    if let Some(live) = base.live_location_message.as_option() {
        let caption = live.caption.clone().unwrap_or_default();
        let text = if caption.is_empty() {
            location_text(None, None, live.degrees_latitude, live.degrees_longitude)
        } else {
            caption
        };
        return Some(Parts::new(MessageKind::Location, text));
    }
    let poll = base
        .poll_creation_message
        .as_option()
        .or_else(|| base.poll_creation_message_v2.as_option())
        .or_else(|| base.poll_creation_message_v3.as_option())
        .or_else(|| base.poll_creation_message_v5.as_option())
        .or_else(|| base.poll_creation_message_v6.as_option());
    if let Some(poll) = poll {
        return Some(Parts::new(MessageKind::Poll, poll_text(poll)));
    }
    if is_noise(base) {
        return None;
    }
    Some(Parts::new(MessageKind::Other, String::new()))
}

fn clip(text: &str) -> String {
    if text.chars().count() <= QUOTE_LIMIT {
        return text.to_string();
    }
    let mut out: String = text.chars().take(QUOTE_LIMIT).collect();
    out.push('…');
    out
}

fn quote_text(quoted: &wa::Message) -> String {
    let base = quoted.get_base_message();
    match classify(base) {
        Some(parts) => match (parts.kind.placeholder(), parts.text.trim().is_empty()) {
            (Some(placeholder), true) => placeholder.to_string(),
            (Some(placeholder), false) => clip(&format!("{placeholder} {}", parts.text)),
            (None, _) => clip(&parts.text),
        },
        None => String::new(),
    }
}

pub fn links(message: &wa::Message) -> Links {
    let base = message.get_base_message();
    let Some(context) = base.context_info() else {
        return Links::default();
    };
    Links {
        quoted_id: context.stanza_id.clone().unwrap_or_default(),
        quoted_participant: context.participant.clone().unwrap_or_default(),
        quoted_text: context
            .quoted_message
            .as_option()
            .map(quote_text)
            .unwrap_or_default(),
        forwarded: context.is_forwarded.unwrap_or(false),
        mentioned: context.mentioned_jid.clone(),
    }
}

pub fn build(meta: &Meta, message: &wa::Message) -> Content {
    let base = message.get_base_message();
    if let Some(protocol) = base.protocol_message.as_option() {
        return protocol_content(meta, protocol);
    }
    if let Some(reaction) = base.reaction_message.as_option() {
        let Some(target) = reaction.key.as_option().and_then(|key| key.id.clone()) else {
            return Content::Skip;
        };
        return Content::Reaction(WireReaction {
            jid: meta.chat.clone(),
            id: target,
            de: meta.sender.clone(),
            minha: meta.from_me,
            emoji: reaction.text.clone().unwrap_or_default(),
            em: meta.ts,
        });
    }
    let Some(parts) = classify(base) else {
        return Content::Skip;
    };
    let linked = links(message);
    let mut text = parts.text;
    for token in &meta.own_tokens {
        text = super::replace_mention(&text, token, "você");
    }
    Content::Message(Box::new(WireMessage {
        id: meta.id.clone(),
        jid: meta.chat.clone(),
        de: meta.sender.clone(),
        nome_de: if meta.from_me {
            String::new()
        } else {
            meta.push_name.clone()
        },
        texto: text,
        minha: meta.from_me,
        em: meta.ts,
        tipo: parts.kind.as_text().to_string(),
        historico: meta.history,
        bruto: waproto::codec::message_to_vec(message),
        autor_bruto: meta.sender_raw.clone(),
        citada_id: linked.quoted_id,
        citada_de: meta.quoted_from.clone(),
        citada_texto: linked.quoted_text,
        encaminhada: linked.forwarded,
        nome_arquivo: parts.file_name,
        mime: parts.mime,
        duracao: parts.duration,
        tamanho: parts.size,
        largura: parts.width,
        altura: parts.height,
        voz: parts.voice,
        mencoes: meta.mentions.clone(),
    }))
}

fn protocol_content(meta: &Meta, protocol: &wa::message::ProtocolMessage) -> Content {
    use wa::message::protocol_message::Type;
    let target = protocol.key.as_option().and_then(|key| key.id.clone());
    match (protocol.r#type, target) {
        (Some(Type::REVOKE), Some(id)) => Content::Revoke(WireRevoke {
            jid: meta.chat.clone(),
            id,
            de: meta.sender.clone(),
            minha: meta.from_me,
            admin: meta.admin_revoke,
            em: meta.ts,
        }),
        (Some(Type::MESSAGE_EDIT), Some(id)) => {
            let Some(edited) = protocol.edited_message.as_option() else {
                return Content::Skip;
            };
            let Some(parts) = classify(edited.get_base_message()) else {
                return Content::Skip;
            };
            Content::Edit(WireEdit {
                jid: meta.chat.clone(),
                id,
                de: meta.sender.clone(),
                minha: meta.from_me,
                texto: parts.text,
                em: meta.ts,
            })
        }
        _ => Content::Skip,
    }
}
