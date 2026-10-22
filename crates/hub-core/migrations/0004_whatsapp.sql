CREATE TABLE wa_conversas (
    jid TEXT PRIMARY KEY,
    nome TEXT NOT NULL DEFAULT '',
    grupo INTEGER NOT NULL DEFAULT 0,
    ultima_em TEXT
);

CREATE TABLE wa_mensagens (
    id TEXT NOT NULL,
    jid TEXT NOT NULL,
    de TEXT NOT NULL DEFAULT '',
    nome_de TEXT NOT NULL DEFAULT '',
    texto TEXT NOT NULL DEFAULT '',
    minha INTEGER NOT NULL DEFAULT 0,
    em TEXT NOT NULL,
    tipo TEXT NOT NULL DEFAULT 'texto',
    PRIMARY KEY (jid, id)
);

CREATE INDEX wa_mensagens_em ON wa_mensagens(jid, em);

CREATE TABLE wa_apelidos (
    jid TEXT PRIMARY KEY,
    apelido TEXT NOT NULL
);
