use anyhow::Context;
use hub_core::{Config, Database, HubPaths};

fn main() -> anyhow::Result<()> {
    let paths = HubPaths::resolve().context("resolvendo as pastas do hub")?;
    paths.ensure_dirs().context("criando as pastas do hub")?;
    let _log_guard = hub_core::logging::init(&paths.logs_dir).context("iniciando os logs")?;
    let config = Config::load_or_create(&paths.config_file()).context("lendo config.toml")?;
    let db = Database::open(&paths.database_file()).context("abrindo o banco de dados")?;
    tracing::info!(
        renderizador = ?config.general.renderer,
        versao_banco = db.schema_version()?,
        "hub iniciado"
    );
    Ok(())
}
