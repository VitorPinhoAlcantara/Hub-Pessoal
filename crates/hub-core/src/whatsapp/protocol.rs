#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMention {
    pub token: String,
    pub jid: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMessage {
    pub id: String,
    pub jid: String,
    pub de: String,
    pub nome_de: String,
    pub texto: String,
    pub minha: bool,
    pub em: i64,
    pub tipo: String,
    pub historico: bool,
    pub bruto: Vec<u8>,
    pub autor_bruto: String,
    pub citada_id: String,
    pub citada_de: String,
    pub citada_texto: String,
    pub encaminhada: bool,
    pub nome_arquivo: String,
    pub mime: String,
    pub duracao: u32,
    pub tamanho: u64,
    pub largura: u32,
    pub altura: u32,
    pub voz: bool,
    pub mencoes: Vec<WireMention>,
}

impl Default for WireMessage {
    fn default() -> Self {
        Self {
            id: String::new(),
            jid: String::new(),
            de: String::new(),
            nome_de: String::new(),
            texto: String::new(),
            minha: false,
            em: 0,
            tipo: "texto".to_string(),
            historico: false,
            bruto: Vec::new(),
            autor_bruto: String::new(),
            citada_id: String::new(),
            citada_de: String::new(),
            citada_texto: String::new(),
            encaminhada: false,
            nome_arquivo: String::new(),
            mime: String::new(),
            duracao: 0,
            tamanho: 0,
            largura: 0,
            altura: 0,
            voz: false,
            mencoes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireEdit {
    pub jid: String,
    pub id: String,
    pub de: String,
    pub minha: bool,
    pub texto: String,
    pub em: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireRevoke {
    pub jid: String,
    pub id: String,
    pub de: String,
    pub minha: bool,
    pub admin: bool,
    pub em: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireReaction {
    pub jid: String,
    pub id: String,
    pub de: String,
    pub minha: bool,
    pub emoji: String,
    pub em: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireConversation {
    pub jid: String,
    pub nome: String,
    pub grupo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireChatInfo {
    pub jid: String,
    pub phone: String,
    pub nome: String,
    pub arquivada: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireParticipant {
    pub jid: String,
    pub nome: String,
}
