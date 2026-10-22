#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod shell;
mod streams;

use std::sync::Arc;

use anyhow::Context;
use hub_core::clock::SystemClock;
use hub_core::config::Renderer;
use hub_core::{Config, Database, HubContext, HubPaths};

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub smoke_test: bool,
    pub start_hidden: bool,
    pub mcp: bool,
}

fn parse_options() -> Options {
    let mut options = Options {
        mcp: true,
        ..Options::default()
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--smoke-test" => options.smoke_test = true,
            "--bandeja" => options.start_hidden = true,
            "--sem-mcp" => options.mcp = false,
            _ => {}
        }
    }
    options
}

fn configure_renderer(renderer: Renderer) {
    let set_default = |key: &str, value: &str| {
        if std::env::var_os(key).is_none() {
            unsafe { std::env::set_var(key, value) };
        }
    };
    if renderer == Renderer::Software {
        set_default("ICED_BACKEND", "tiny-skia");
    }
    if let Some(backend) = hub_core::platform::preferred_gpu_backend() {
        set_default("WGPU_BACKEND", backend);
    }
}

fn main() -> anyhow::Result<()> {
    let options = parse_options();
    let paths = HubPaths::resolve().context("resolvendo as pastas do hub")?;
    paths.ensure_dirs().context("criando as pastas do hub")?;
    let config = Config::load_or_create(&paths.config_file()).context("lendo config.toml")?;
    configure_renderer(config.general.renderer);
    let _log_guard = hub_core::logging::init(&paths.logs_dir).context("iniciando os logs")?;
    let db = Database::open(&paths.database_file()).context("abrindo o banco de dados")?;
    let ctx = HubContext::new(
        paths,
        config,
        db,
        hub_core::platform::native(),
        Arc::new(SystemClock),
    );
    tracing::info!(?options, "hub iniciando");
    app::run(ctx, options).context("executando a interface")?;
    tracing::info!("hub encerrado");
    Ok(())
}
