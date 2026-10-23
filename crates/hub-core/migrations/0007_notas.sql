CREATE TABLE notas (
    id INTEGER PRIMARY KEY,
    titulo TEXT NOT NULL DEFAULT '',
    conteudo TEXT NOT NULL DEFAULT '',
    favorita INTEGER NOT NULL DEFAULT 0,
    reset TEXT NOT NULL DEFAULT '',
    criada_em TEXT NOT NULL,
    atualizada_em TEXT NOT NULL,
    ultimo_reset TEXT
);
