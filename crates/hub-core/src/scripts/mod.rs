pub mod runner;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::per_os::PerOs;
use crate::platform::Os;

pub const OUTPUT_LIMIT: usize = 64 * 1024;
pub const DEFAULT_TIMEOUT: u64 = 300;

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("erro no banco de dados: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("erro de arquivo: {0}")]
    Io(#[from] std::io::Error),
    #[error("falha ao gravar os metadados: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("script {0} não encontrado")]
    NotFound(String),
    #[error("já existe um script com o nome {0}")]
    AlreadyExists(String),
    #[error("o nome do script não pode ficar vazio")]
    EmptyName,
    #[error("informe ao menos um arquivo (Windows ou Linux)")]
    NoFile,
    #[error("agendamento inválido: {0}")]
    InvalidSchedule(String),
}

pub type Result<T> = std::result::Result<T, ScriptError>;

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptMeta {
    #[serde(rename = "nome")]
    pub name: String,
    #[serde(rename = "descricao", default)]
    pub description: String,
    #[serde(rename = "arquivo", default)]
    pub file: PerOs,
    #[serde(rename = "confirmar_antes", default)]
    pub confirm: bool,
    #[serde(rename = "timeout_segundos", default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(
        rename = "agendamento",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub schedule: Option<String>,
    #[serde(rename = "ativo", default = "default_true")]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub id: String,
    pub meta: ScriptMeta,
    pub meta_path: PathBuf,
    pub file: Option<PathBuf>,
    pub problem: Option<String>,
}

impl Script {
    pub fn can_run(&self) -> bool {
        self.file.is_some() && self.problem.is_none()
    }

    pub fn is_scheduled(&self) -> bool {
        self.meta.active && self.can_run() && self.meta.schedule.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewScript {
    pub name: String,
    pub description: String,
    pub windows_file: Option<String>,
    pub linux_file: Option<String>,
    pub windows_content: String,
    pub linux_content: String,
    pub confirm: bool,
    pub timeout_secs: u64,
    pub schedule: Option<String>,
    pub active: bool,
}

#[derive(Default)]
pub struct ScriptStore {
    last_valid: HashMap<PathBuf, ScriptMeta>,
}

pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        let mapped = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        if mapped == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(mapped);
    }
    out.trim_end_matches('-').to_string()
}

pub fn parse_schedule(text: &str) -> Result<cron::Schedule> {
    let text = text.trim();
    let fields = text.split_whitespace().count();
    let normalized = if fields == 5 {
        format!("0 {text}")
    } else {
        text.to_string()
    };
    cron::Schedule::from_str(&normalized)
        .map_err(|error| ScriptError::InvalidSchedule(format!("{text}: {error}")))
}

pub fn next_run(schedule: &str, after: NaiveDateTime) -> Option<NaiveDateTime> {
    let schedule = parse_schedule(schedule).ok()?;
    schedule
        .after(&Utc.from_utc_datetime(&after))
        .next()
        .map(|at| at.naive_utc())
}

impl ScriptStore {
    pub fn list(&mut self, dir: &Path, os: Os) -> Vec<Script> {
        let mut scripts = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return scripts;
        };
        let mut metas: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        metas.sort();
        for meta_path in metas {
            let id = meta_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parsed = std::fs::read_to_string(&meta_path)
                .map_err(|e| e.to_string())
                .and_then(|text| toml::from_str::<ScriptMeta>(&text).map_err(|e| e.to_string()));
            let (meta, mut problem) = match parsed {
                Ok(meta) => {
                    self.last_valid.insert(meta_path.clone(), meta.clone());
                    (meta, None)
                }
                Err(error) => {
                    tracing::warn!(arquivo = %meta_path.display(), %error, "metadados de script inválidos");
                    match self.last_valid.get(&meta_path) {
                        Some(meta) => (
                            meta.clone(),
                            Some(format!(
                                "Arquivo inválido, usando a última versão válida: {error}"
                            )),
                        ),
                        None => (
                            ScriptMeta {
                                name: id.clone(),
                                description: String::new(),
                                file: PerOs::default(),
                                confirm: false,
                                timeout_secs: DEFAULT_TIMEOUT,
                                schedule: None,
                                active: false,
                            },
                            Some(format!("Arquivo inválido: {error}")),
                        ),
                    }
                }
            };
            let file = meta.file.for_os(os).map(|name| dir.join(name));
            let file = match file {
                Some(path) if path.is_file() => Some(path),
                Some(path) => {
                    problem.get_or_insert(format!("Arquivo {} não encontrado", path.display()));
                    None
                }
                None => {
                    problem.get_or_insert("Sem versão para este sistema".to_string());
                    None
                }
            };
            if let Some(schedule) = &meta.schedule
                && let Err(error) = parse_schedule(schedule)
            {
                problem.get_or_insert(error.to_string());
            }
            scripts.push(Script {
                id,
                meta,
                meta_path,
                file,
                problem,
            });
        }
        scripts
    }

    pub fn get(&mut self, dir: &Path, os: Os, id: &str) -> Result<Script> {
        self.list(dir, os)
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| ScriptError::NotFound(id.to_string()))
    }
}

pub fn create(dir: &Path, script: NewScript) -> Result<String> {
    let name = script.name.trim();
    if name.is_empty() {
        return Err(ScriptError::EmptyName);
    }
    let id = slug(name);
    if id.is_empty() {
        return Err(ScriptError::EmptyName);
    }
    let meta_path = dir.join(format!("{id}.toml"));
    if meta_path.exists() {
        return Err(ScriptError::AlreadyExists(name.to_string()));
    }
    let file = PerOs::split(script.windows_file, script.linux_file);
    if file.windows().is_none() && file.linux().is_none() {
        return Err(ScriptError::NoFile);
    }
    if let Some(schedule) = &script.schedule {
        parse_schedule(schedule)?;
    }
    std::fs::create_dir_all(dir)?;
    for (name, content) in [
        (file.windows(), &script.windows_content),
        (file.linux(), &script.linux_content),
    ] {
        if let Some(name) = name {
            let path = dir.join(name);
            if !path.exists() {
                std::fs::write(&path, content)?;
            }
        }
    }
    let meta = ScriptMeta {
        name: name.to_string(),
        description: script.description.trim().to_string(),
        file,
        confirm: script.confirm,
        timeout_secs: if script.timeout_secs == 0 {
            DEFAULT_TIMEOUT
        } else {
            script.timeout_secs
        },
        schedule: script.schedule.filter(|s| !s.trim().is_empty()),
        active: script.active,
    };
    std::fs::write(&meta_path, toml::to_string_pretty(&meta)?)?;
    Ok(id)
}

pub fn save_meta(script: &Script, meta: &ScriptMeta) -> Result<()> {
    if meta.name.trim().is_empty() {
        return Err(ScriptError::EmptyName);
    }
    if let Some(schedule) = &meta.schedule {
        parse_schedule(schedule)?;
    }
    std::fs::write(&script.meta_path, toml::to_string_pretty(meta)?)?;
    Ok(())
}

pub fn delete(dir: &Path, script: &Script) -> Result<()> {
    std::fs::remove_file(&script.meta_path)?;
    for name in [script.meta.file.windows(), script.meta.file.linux()]
        .into_iter()
        .flatten()
    {
        let path = dir.join(name);
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Manual,
    Scheduled,
    Ai,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Manual => "manual",
            Origin::Scheduled => "agendada",
            Origin::Ai => "ia",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "agendada" => Origin::Scheduled,
            "ia" => Origin::Ai,
            _ => Origin::Manual,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Origin::Manual => "manual",
            Origin::Scheduled => "agendada",
            Origin::Ai => "IA",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execution {
    pub id: i64,
    pub script: String,
    pub started_at: NaiveDateTime,
    pub finished_at: Option<NaiveDateTime>,
    pub exit_code: Option<i32>,
    pub output: String,
    pub origin: Origin,
}

impl Execution {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

fn execution_from_row(row: &Row<'_>) -> rusqlite::Result<Execution> {
    let origin: String = row.get(6)?;
    Ok(Execution {
        id: row.get(0)?,
        script: row.get(1)?,
        started_at: row.get(2)?,
        finished_at: row.get(3)?,
        exit_code: row.get(4)?,
        output: row.get(5)?,
        origin: Origin::parse(&origin),
    })
}

pub fn start_execution(
    conn: &Connection,
    script: &str,
    origin: Origin,
    now: NaiveDateTime,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO execucoes (script, inicio, origem) VALUES (?1, ?2, ?3)",
        params![script, now, origin.as_str()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_execution(
    conn: &Connection,
    id: i64,
    exit_code: Option<i32>,
    output: &str,
    now: NaiveDateTime,
) -> Result<()> {
    conn.execute(
        "UPDATE execucoes SET fim = ?2, codigo_saida = ?3, saida = ?4 WHERE id = ?1",
        params![id, now, exit_code, truncate_output(output)],
    )?;
    Ok(())
}

pub fn executions(conn: &Connection, script: &str, limit: usize) -> Result<Vec<Execution>> {
    let mut stmt = conn.prepare(
        "SELECT id, script, inicio, fim, codigo_saida, saida, origem FROM execucoes
         WHERE script = ?1 ORDER BY inicio DESC, id DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![script, limit as i64], execution_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn last_executions(conn: &Connection) -> Result<HashMap<String, Execution>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.script, e.inicio, e.fim, e.codigo_saida, e.saida, e.origem FROM execucoes e
         WHERE e.id = (SELECT MAX(id) FROM execucoes x WHERE x.script = e.script)",
    )?;
    let rows = stmt
        .query_map([], execution_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().map(|e| (e.script.clone(), e)).collect())
}

pub fn truncate_output(output: &str) -> &str {
    if output.len() <= OUTPUT_LIMIT {
        return output;
    }
    let mut start = output.len() - OUTPUT_LIMIT;
    while !output.is_char_boundary(start) {
        start += 1;
    }
    &output[start..]
}

pub fn due_between(scripts: &[Script], from: NaiveDateTime, to: NaiveDateTime) -> Vec<Script> {
    scripts
        .iter()
        .filter(|s| s.is_scheduled())
        .filter(|s| {
            s.meta
                .schedule
                .as_deref()
                .and_then(|schedule| next_run(schedule, from))
                .is_some_and(|at| at <= to)
        })
        .cloned()
        .collect()
}

pub fn next_scheduled(scripts: &[Script], after: NaiveDateTime) -> Option<NaiveDateTime> {
    scripts
        .iter()
        .filter(|s| s.is_scheduled())
        .filter_map(|s| next_run(s.meta.schedule.as_deref()?, after))
        .min()
}

pub fn template(file: &str) -> String {
    let extension = Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "ps1" => "Write-Output \"Olá do hub\"\n".to_string(),
        "bat" | "cmd" => "@echo off\r\necho Olá do hub\r\n".to_string(),
        "sh" => "#!/bin/sh\necho \"Olá do hub\"\n".to_string(),
        "py" => "print(\"Olá do hub\")\n".to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;
