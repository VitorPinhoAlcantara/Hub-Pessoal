use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::{Connection, OpenFlags};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::paths::HubPaths;

const FILE_PREFIX: &str = "hub-backup-";
const FILE_EXTENSION: &str = ".zip";
const STAMP_FORMAT: &str = "%Y%m%d-%H%M%S";
const STAMP_LEN: usize = 15;
const MANIFEST_NAME: &str = "manifesto.json";
const PENDING_FILE: &str = "restaurar.pendente";
const DATABASES: [&str; 2] = ["hub.db", "whatsapp-sessao.db"];
const KEEP_UPDATE: usize = 3;
const KEEP_BEFORE_RESTORE: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("erro de entrada e saída: {0}")]
    Io(#[from] std::io::Error),
    #[error("falha no banco de dados: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("falha no arquivo zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("falha ao gerar o manifesto: {0}")]
    Json(#[from] serde_json::Error),
    #[error("o arquivo {0} não é um backup válido do hub")]
    Invalid(PathBuf),
    #[error("o backup {0} não foi encontrado")]
    Missing(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackupReason {
    Daily,
    Manual,
    Update,
    BeforeRestore,
}

impl BackupReason {
    pub const ALL: [BackupReason; 4] = [
        BackupReason::Daily,
        BackupReason::Manual,
        BackupReason::Update,
        BackupReason::BeforeRestore,
    ];

    pub fn suffix(self) -> &'static str {
        match self {
            Self::Daily => "diario",
            Self::Manual => "manual",
            Self::Update => "atualizacao",
            Self::BeforeRestore => "antes-de-restaurar",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "Diário",
            Self::Manual => "Manual",
            Self::Update => "Antes de atualizar",
            Self::BeforeRestore => "Antes de restaurar",
        }
    }

    pub fn from_suffix(suffix: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.suffix() == suffix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub created: NaiveDateTime,
    pub reason: BackupReason,
    pub size: u64,
}

pub fn file_name(reason: BackupReason, created: NaiveDateTime) -> String {
    format!(
        "{FILE_PREFIX}{}-{}{FILE_EXTENSION}",
        created.format(STAMP_FORMAT),
        reason.suffix()
    )
}

fn parse_file_name(name: &str) -> Option<(NaiveDateTime, BackupReason)> {
    let body = name
        .strip_prefix(FILE_PREFIX)?
        .strip_suffix(FILE_EXTENSION)?;
    let stamp = body.get(..STAMP_LEN)?;
    let suffix = body.get(STAMP_LEN..)?.strip_prefix('-')?;
    let created = NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT).ok()?;
    Some((created, BackupReason::from_suffix(suffix)?))
}

fn sql_quote(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn snapshot_database(source: &Path, target: &Path) -> Result<(), BackupError> {
    let connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(10))?;
    connection.execute_batch(&format!("VACUUM INTO '{}'", sql_quote(target)))?;
    Ok(())
}

fn add_file<W: Write + std::io::Seek>(
    writer: &mut ZipWriter<W>,
    entry: &str,
    source: &Path,
) -> Result<(), BackupError> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(entry, options)?;
    let mut file = File::open(source)?;
    std::io::copy(&mut file, writer)?;
    Ok(())
}

fn add_tree<W: Write + std::io::Seek>(
    writer: &mut ZipWriter<W>,
    root: &Path,
    entry_root: &str,
) -> Result<(), BackupError> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let parts: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            add_file(writer, &format!("{entry_root}/{}", parts.join("/")), &path)?;
        }
    }
    Ok(())
}

fn write_archive(
    paths: &HubPaths,
    reason: BackupReason,
    now: NaiveDateTime,
    partial: &Path,
    scratch: &Path,
) -> Result<(), BackupError> {
    let mut writer = ZipWriter::new(File::create(partial)?);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let manifest = serde_json::json!({
        "versao": crate::channel::VERSION,
        "canal": match crate::channel::Channel::current() {
            crate::channel::Channel::Dev => "dev",
            crate::channel::Channel::Release => "distribuicao",
        },
        "criado_em": now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "motivo": reason.suffix(),
    });
    writer.start_file(MANIFEST_NAME, options)?;
    writer.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    add_tree(&mut writer, &paths.config_dir, "config")?;
    for name in DATABASES {
        let source = paths.data_dir.join(name);
        if source.is_file() {
            let copy = scratch.join(name);
            snapshot_database(&source, &copy)?;
            add_file(&mut writer, &format!("data/{name}"), &copy)?;
        }
    }
    let mcp = paths.data_dir.join("mcp.json");
    if mcp.is_file() {
        add_file(&mut writer, "data/mcp.json", &mcp)?;
    }
    writer.finish()?.sync_all()?;
    Ok(())
}

pub fn create(
    paths: &HubPaths,
    reason: BackupReason,
    now: NaiveDateTime,
) -> Result<BackupInfo, BackupError> {
    let dir = paths.backups_dir();
    std::fs::create_dir_all(&dir)?;
    let name = file_name(reason, now);
    let target = dir.join(&name);
    let partial = dir.join(format!("{name}.parcial"));
    let scratch = dir.join(format!("{name}.tmp"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let result = write_archive(paths, reason, now, &partial, &scratch)
        .and_then(|()| std::fs::rename(&partial, &target).map_err(BackupError::from));
    let _ = std::fs::remove_dir_all(&scratch);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    let size = std::fs::metadata(&target)?.len();
    Ok(BackupInfo {
        path: target,
        created: now,
        reason,
        size,
    })
}

pub fn list(paths: &HubPaths) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(paths.backups_dir()) else {
        return Vec::new();
    };
    let mut found: Vec<BackupInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let (created, reason) = parse_file_name(&name)?;
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| BackupInfo {
                path: entry.path(),
                created,
                reason,
                size: metadata.len(),
            })
        })
        .collect();
    found.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.path.cmp(&a.path)));
    found
}

pub fn prune(paths: &HubPaths, keep_daily: usize) {
    let backups = list(paths);
    for (reason, keep) in [
        (BackupReason::Daily, keep_daily),
        (BackupReason::Update, KEEP_UPDATE),
        (BackupReason::BeforeRestore, KEEP_BEFORE_RESTORE),
    ] {
        for backup in backups.iter().filter(|b| b.reason == reason).skip(keep) {
            if let Err(error) = std::fs::remove_file(&backup.path) {
                tracing::warn!(path = %backup.path.display(), %error, "falha ao apagar backup antigo");
            }
        }
    }
}

pub fn needs_daily(paths: &HubPaths, today: NaiveDate) -> bool {
    !list(paths)
        .iter()
        .any(|backup| backup.reason == BackupReason::Daily && backup.created.date() == today)
}

fn open_archive(zip: &Path) -> Result<ZipArchive<File>, BackupError> {
    if !zip.is_file() {
        return Err(BackupError::Missing(zip.to_path_buf()));
    }
    let mut archive =
        ZipArchive::new(File::open(zip)?).map_err(|_| BackupError::Invalid(zip.to_path_buf()))?;
    if archive.by_name(MANIFEST_NAME).is_err() {
        return Err(BackupError::Invalid(zip.to_path_buf()));
    }
    Ok(archive)
}

fn pending_file(paths: &HubPaths) -> PathBuf {
    paths.data_dir.join(PENDING_FILE)
}

pub fn schedule_restore(paths: &HubPaths, zip: &Path) -> Result<(), BackupError> {
    open_archive(zip)?;
    std::fs::create_dir_all(&paths.data_dir)?;
    std::fs::write(pending_file(paths), zip.to_string_lossy().as_bytes())?;
    Ok(())
}

pub fn has_pending_restore(paths: &HubPaths) -> bool {
    pending_file(paths).is_file()
}

pub fn cancel_pending_restore(paths: &HubPaths) {
    let _ = std::fs::remove_file(pending_file(paths));
}

fn extract(archive: &mut ZipArchive<File>, paths: &HubPaths) -> Result<(), BackupError> {
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let target = if let Ok(rest) = name.strip_prefix("data") {
            paths.data_dir.join(rest)
        } else if let Ok(rest) = name.strip_prefix("config") {
            paths.config_dir.join(rest)
        } else {
            continue;
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&target)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

pub fn apply_pending_restore(
    paths: &HubPaths,
    now: NaiveDateTime,
) -> Result<Option<PathBuf>, BackupError> {
    let pending = pending_file(paths);
    let Ok(text) = std::fs::read_to_string(&pending) else {
        return Ok(None);
    };
    let zip = PathBuf::from(text.trim());
    let result = restore_from(paths, &zip, now);
    let _ = std::fs::remove_file(&pending);
    result.map(|()| Some(zip))
}

fn restore_from(paths: &HubPaths, zip: &Path, now: NaiveDateTime) -> Result<(), BackupError> {
    let mut archive = open_archive(zip)?;
    create(paths, BackupReason::BeforeRestore, now)?;
    for name in DATABASES {
        for suffix in ["", "-wal", "-shm"] {
            let file = paths.data_dir.join(format!("{name}{suffix}"));
            if file.exists() {
                std::fs::remove_file(&file)?;
            }
        }
    }
    extract(&mut archive, paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(day: u32, hour: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(hour, 30, 5)
            .unwrap()
    }

    fn setup() -> (tempfile::TempDir, HubPaths) {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path());
        paths.ensure_dirs().unwrap();
        (temp, paths)
    }

    fn write_value(paths: &HubPaths, value: &str) {
        let connection = Connection::open(paths.database_file()).unwrap();
        connection
            .execute_batch("CREATE TABLE IF NOT EXISTS item (valor TEXT); DELETE FROM item;")
            .unwrap();
        connection
            .execute("INSERT INTO item (valor) VALUES (?1)", [value])
            .unwrap();
    }

    fn read_value(paths: &HubPaths) -> String {
        let connection = Connection::open(paths.database_file()).unwrap();
        connection
            .query_row("SELECT valor FROM item", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn file_names_round_trip() {
        for reason in BackupReason::ALL {
            let name = file_name(reason, moment(3, 9));
            assert_eq!(parse_file_name(&name), Some((moment(3, 9), reason)));
        }
        assert_eq!(
            file_name(BackupReason::Daily, moment(3, 9)),
            "hub-backup-20261003-093005-diario.zip"
        );
        assert_eq!(parse_file_name("hub-backup-xx-diario.zip"), None);
        assert_eq!(parse_file_name("outro.zip"), None);
        assert_eq!(
            parse_file_name("hub-backup-20261003-093005-qualquer.zip"),
            None
        );
    }

    #[test]
    fn create_builds_zip_with_expected_entries() {
        let (_temp, paths) = setup();
        write_value(&paths, "dado");
        std::fs::write(paths.config_file(), "x = 1").unwrap();
        std::fs::write(paths.scripts_dir.join("a.ps1"), "echo").unwrap();
        std::fs::write(paths.data_dir.join("mcp.json"), "{}").unwrap();
        std::fs::write(paths.logs_dir.join("hub.log"), "log").unwrap();
        let info = create(&paths, BackupReason::Manual, moment(3, 9)).unwrap();
        assert!(info.path.is_file());
        assert!(info.size > 0);
        assert_eq!(info.reason, BackupReason::Manual);
        let mut archive = ZipArchive::new(File::open(&info.path).unwrap()).unwrap();
        let mut names: Vec<String> = archive
            .file_names()
            .filter_map(|name| name.ok().map(|name| name.into_owned()))
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "config/config.toml",
                "config/scripts/a.ps1",
                "data/hub.db",
                "data/mcp.json",
                "manifesto.json"
            ]
        );
        let mut manifest = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("manifesto.json").unwrap(),
            &mut manifest,
        )
        .unwrap();
        assert!(manifest.contains("\"motivo\": \"manual\""));
        let leftovers: Vec<_> = std::fs::read_dir(paths.backups_dir())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(leftovers.len(), 1);
    }

    #[test]
    fn list_is_newest_first_and_ignores_strangers() {
        let (_temp, paths) = setup();
        create(&paths, BackupReason::Daily, moment(1, 8)).unwrap();
        create(&paths, BackupReason::Manual, moment(3, 8)).unwrap();
        create(&paths, BackupReason::Update, moment(2, 8)).unwrap();
        std::fs::write(paths.backups_dir().join("lixo.zip"), "x").unwrap();
        std::fs::write(
            paths
                .backups_dir()
                .join("hub-backup-20261001-000000-diario.zip.parcial"),
            "x",
        )
        .unwrap();
        let found = list(&paths);
        let days: Vec<u32> = found
            .iter()
            .map(|b| chrono::Datelike::day(&b.created))
            .collect();
        assert_eq!(days, vec![3, 2, 1]);
    }

    #[test]
    fn prune_keeps_limits_and_never_touches_manual() {
        let (_temp, paths) = setup();
        for day in 1..=5 {
            create(&paths, BackupReason::Daily, moment(day, 8)).unwrap();
        }
        for day in 1..=5 {
            create(&paths, BackupReason::Update, moment(day, 9)).unwrap();
            create(&paths, BackupReason::BeforeRestore, moment(day, 10)).unwrap();
            create(&paths, BackupReason::Manual, moment(day, 11)).unwrap();
        }
        prune(&paths, 2);
        let found = list(&paths);
        let count = |reason| found.iter().filter(|b| b.reason == reason).count();
        assert_eq!(count(BackupReason::Daily), 2);
        assert_eq!(count(BackupReason::Update), 3);
        assert_eq!(count(BackupReason::BeforeRestore), 3);
        assert_eq!(count(BackupReason::Manual), 5);
        let newest_daily = found
            .iter()
            .find(|b| b.reason == BackupReason::Daily)
            .unwrap();
        assert_eq!(newest_daily.created, moment(5, 8));
    }

    #[test]
    fn needs_daily_checks_today_only() {
        let (_temp, paths) = setup();
        let today = moment(3, 8).date();
        assert!(needs_daily(&paths, today));
        create(&paths, BackupReason::Manual, moment(3, 8)).unwrap();
        assert!(needs_daily(&paths, today));
        create(&paths, BackupReason::Daily, moment(2, 8)).unwrap();
        assert!(needs_daily(&paths, today));
        create(&paths, BackupReason::Daily, moment(3, 9)).unwrap();
        assert!(!needs_daily(&paths, today));
    }

    #[test]
    fn restore_brings_back_old_content() {
        let (_temp, paths) = setup();
        write_value(&paths, "antigo");
        std::fs::write(paths.config_file(), "versao = 1").unwrap();
        let backup = create(&paths, BackupReason::Manual, moment(3, 8)).unwrap();
        write_value(&paths, "novo");
        std::fs::write(paths.config_file(), "versao = 2").unwrap();
        assert_eq!(read_value(&paths), "novo");

        assert!(!has_pending_restore(&paths));
        schedule_restore(&paths, &backup.path).unwrap();
        assert!(has_pending_restore(&paths));
        let restored = apply_pending_restore(&paths, moment(4, 8)).unwrap();
        assert_eq!(restored, Some(backup.path.clone()));
        assert!(!has_pending_restore(&paths));
        assert_eq!(read_value(&paths), "antigo");
        assert_eq!(
            std::fs::read_to_string(paths.config_file()).unwrap(),
            "versao = 1"
        );
        let found = list(&paths);
        assert!(
            found
                .iter()
                .any(|b| b.reason == BackupReason::BeforeRestore)
        );
        assert_eq!(apply_pending_restore(&paths, moment(4, 9)).unwrap(), None);
    }

    #[test]
    fn schedule_rejects_zip_without_manifest() {
        let (temp, paths) = setup();
        let fake = temp.path().join("falso.zip");
        let mut writer = ZipWriter::new(File::create(&fake).unwrap());
        writer
            .start_file("qualquer.txt", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        assert!(matches!(
            schedule_restore(&paths, &fake),
            Err(BackupError::Invalid(_))
        ));
        assert!(matches!(
            schedule_restore(&paths, &temp.path().join("nao-existe.zip")),
            Err(BackupError::Missing(_))
        ));
    }

    #[test]
    fn pending_restore_with_missing_zip_is_cleared_with_error() {
        let (temp, paths) = setup();
        std::fs::write(
            pending_file(&paths),
            temp.path().join("sumiu.zip").to_string_lossy().as_bytes(),
        )
        .unwrap();
        assert!(matches!(
            apply_pending_restore(&paths, moment(4, 8)),
            Err(BackupError::Missing(_))
        ));
        assert!(!has_pending_restore(&paths));
    }
}
