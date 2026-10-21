CREATE TABLE rotinas (
    id INTEGER PRIMARY KEY,
    titulo TEXT NOT NULL,
    notas TEXT NOT NULL DEFAULT '',
    recorrencia TEXT NOT NULL,
    hora TEXT,
    ativa INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE tarefas (
    id INTEGER PRIMARY KEY,
    titulo TEXT NOT NULL,
    notas TEXT NOT NULL DEFAULT '',
    data TEXT,
    hora TEXT,
    prioridade TEXT NOT NULL DEFAULT 'normal' CHECK (prioridade IN ('baixa', 'normal', 'alta')),
    concluida_em TEXT,
    criada_em TEXT NOT NULL,
    rotina_id INTEGER REFERENCES rotinas(id) ON DELETE SET NULL
);

CREATE INDEX tarefas_data ON tarefas(data);

CREATE TABLE lembretes (
    id INTEGER PRIMARY KEY,
    tarefa_id INTEGER REFERENCES tarefas(id) ON DELETE CASCADE,
    titulo TEXT,
    dispara_em TEXT NOT NULL,
    disparado_em TEXT,
    adiado_ate TEXT
);

CREATE INDEX lembretes_dispara_em ON lembretes(dispara_em);

CREATE TABLE execucoes (
    id INTEGER PRIMARY KEY,
    script TEXT NOT NULL,
    inicio TEXT NOT NULL,
    fim TEXT,
    codigo_saida INTEGER,
    saida TEXT NOT NULL DEFAULT '',
    origem TEXT NOT NULL CHECK (origem IN ('manual', 'agendada', 'ia'))
);

CREATE INDEX execucoes_script ON execucoes(script, inicio);
