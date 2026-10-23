CREATE TABLE atalhos_uso (
    chave TEXT PRIMARY KEY,
    vezes INTEGER NOT NULL DEFAULT 0,
    ultimo_em TEXT NOT NULL
);
