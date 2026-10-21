use std::path::Path;

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_inicial.sql"),
    include_str!("../migrations/0002_notas_lembretes.sql"),
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("erro no banco de dados: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("o banco está na versão {found}, mais nova que a suportada ({supported})")]
    NewerSchema { found: usize, supported: usize },
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn schema_version(&self) -> Result<usize, DbError> {
        schema_version(&self.conn)
    }
}

pub fn latest_version() -> usize {
    MIGRATIONS.len()
}

fn schema_version(conn: &Connection) -> Result<usize, DbError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(usize::try_from(version).unwrap_or(0))
}

fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let current = schema_version(conn)?;
    if current > MIGRATIONS.len() {
        return Err(DbError::NewerSchema {
            found: current,
            supported: MIGRATIONS.len(),
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
        tracing::info!(versao = index + 1, "migração aplicada");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_database_reaches_latest_version() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), latest_version());
    }

    #[test]
    fn migrations_create_expected_tables() {
        let db = Database::open_in_memory().unwrap();
        let mut stmt = db
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for table in ["execucoes", "lembretes", "rotinas", "tarefas"] {
            assert!(tables.iter().any(|t| t == table), "faltou {table}");
        }
    }

    #[test]
    fn reopening_file_does_not_reapply_migrations() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hub.db");
        {
            let db = Database::open(&path).unwrap();
            db.conn()
                .execute(
                    "INSERT INTO tarefas (titulo, criada_em) VALUES ('teste', '2026-10-21T10:00:00')",
                    [],
                )
                .unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), latest_version());
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM tarefas", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn newer_schema_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hub.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 999).unwrap();
        }
        assert!(matches!(
            Database::open(&path),
            Err(DbError::NewerSchema { found: 999, .. })
        ));
    }
}
