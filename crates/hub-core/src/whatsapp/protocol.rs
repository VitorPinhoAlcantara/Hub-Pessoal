use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMessage {
    pub id: String,
    pub jid: String,
    #[serde(default)]
    pub de: String,
    #[serde(default)]
    pub nome_de: String,
    #[serde(default)]
    pub texto: String,
    #[serde(default)]
    pub minha: bool,
    pub em: i64,
    #[serde(default = "default_kind")]
    pub tipo: String,
    #[serde(default)]
    pub historico: bool,
}

fn default_kind() -> String {
    "texto".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireConversation {
    pub jid: String,
    #[serde(default)]
    pub nome: String,
    #[serde(default)]
    pub grupo: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Login,
    ListConversations,
    ListMessages { jid: String, limit: usize },
    Send { jid: String, text: String },
    Shutdown,
}

pub fn encode(request: &Request, id: u64) -> String {
    let value = match request {
        Request::Login => json!({ "id": id, "cmd": "login" }),
        Request::ListConversations => json!({ "id": id, "cmd": "listar_conversas" }),
        Request::ListMessages { jid, limit } => {
            json!({ "id": id, "cmd": "listar_mensagens", "jid": jid, "limite": limit })
        }
        Request::Send { jid, text } => {
            json!({ "id": id, "cmd": "enviar", "jid": jid, "texto": text })
        }
        Request::Shutdown => json!({ "id": id, "cmd": "encerrar" }),
    };
    format!("{value}\n")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    Qr(String),
    Connected(String),
    Message(WireMessage),
    Disconnected(String),
    Error(String),
}

pub fn parse(line: &str) -> Option<Incoming> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if let Some(id) = value.get("id").and_then(Value::as_u64)
        && value.get("ok").is_some()
    {
        let result = if value["ok"].as_bool().unwrap_or(false) {
            Ok(value.get("dados").cloned().unwrap_or(Value::Null))
        } else {
            Err(value
                .get("erro")
                .and_then(Value::as_str)
                .unwrap_or("erro desconhecido no WhatsApp")
                .to_string())
        };
        return Some(Incoming::Response { id, result });
    }
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match value.get("evento").and_then(Value::as_str)? {
        "qr" => Some(Incoming::Qr(text("codigo"))),
        "conectado" => Some(Incoming::Connected(text("jid"))),
        "mensagem_recebida" => {
            let mut message: WireMessage =
                serde_json::from_value(value.get("mensagem")?.clone()).ok()?;
            if value.get("historico").and_then(Value::as_bool) == Some(true) {
                message.historico = true;
            }
            Some(Incoming::Message(message))
        }
        "desconectado" => Some(Incoming::Disconnected(text("motivo"))),
        "erro" => Some(Incoming::Error(text("mensagem"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_json_lines() {
        let line = encode(
            &Request::Send {
                jid: "5511@s.whatsapp.net".into(),
                text: "oi".into(),
            },
            9,
        );
        assert!(line.ends_with('\n'));
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["cmd"], "enviar");
        assert_eq!(value["id"], 9);
        assert_eq!(value["texto"], "oi");
        let list: Value = serde_json::from_str(
            encode(
                &Request::ListMessages {
                    jid: "x".into(),
                    limit: 50,
                },
                1,
            )
            .trim(),
        )
        .unwrap();
        assert_eq!(list["limite"], 50);
    }

    #[test]
    fn responses_and_events_are_parsed() {
        assert_eq!(
            parse(r#"{"id":3,"ok":true,"dados":[1]}"#),
            Some(Incoming::Response {
                id: 3,
                result: Ok(json!([1]))
            })
        );
        assert_eq!(
            parse(r#"{"id":4,"ok":false,"erro":"sem sessão"}"#),
            Some(Incoming::Response {
                id: 4,
                result: Err("sem sessão".into())
            })
        );
        assert_eq!(
            parse(r#"{"evento":"qr","codigo":"2@abc"}"#),
            Some(Incoming::Qr("2@abc".into()))
        );
        assert_eq!(
            parse(r#"{"evento":"desconectado","motivo":"sessao_encerrada"}"#),
            Some(Incoming::Disconnected("sessao_encerrada".into()))
        );
        let Some(Incoming::Message(message)) = parse(
            r#"{"evento":"mensagem_recebida","historico":true,"mensagem":{"id":"A1","jid":"g@g.us","de":"5511@s.whatsapp.net","nome_de":"Ana","texto":"oi","minha":false,"em":1792627000,"tipo":"texto"}}"#,
        ) else {
            panic!("esperava mensagem")
        };
        assert_eq!(message.nome_de, "Ana");
        assert!(message.historico);
        assert!(parse("lixo").is_none());
        assert!(parse(r#"{"evento":"desconhecido"}"#).is_none());
    }
}
