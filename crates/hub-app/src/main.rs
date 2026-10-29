#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_icon;
mod appearance;
mod maintenance;
mod shell;
mod streams;

use std::sync::Arc;

use anyhow::Context;
use hub_core::clock::SystemClock;
use hub_core::config::Renderer;
use hub_core::events::{EventBus, HubEvent};
use hub_core::platform::ipc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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

fn configure_renderer(renderer: Renderer, translucent: bool) {
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
    if translucent && renderer == Renderer::Gpu {
        set_default("WGPU_DX12_PRESENTATION_SYSTEM", "Visual");
    }
}

fn instance_path() -> String {
    ipc::ipc_path(&format!(
        "{}-instancia",
        hub_core::channel::Channel::current().dir_name()
    ))
}

fn notify_running_instance(path: &str, show: bool) -> bool {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    runtime.block_on(async {
        let Ok(mut stream) = ipc::connect(path).await else {
            return false;
        };
        let line: &[u8] = if show { b"abrir
" } else { b"
" };
        let _ = stream.write_all(line).await;
        true
    })
}

fn listen_for_instances(path: String, events: EventBus) {
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            let mut listener = match ipc::IpcListener::bind(&path) {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::warn!(%error, "sem canal de instância única");
                    return;
                }
            };
            while let Ok(stream) = listener.accept().await {
                let mut line = String::new();
                let _ = BufReader::new(stream).read_line(&mut line).await;
                if line.trim() == "abrir" {
                    events.publish(HubEvent::ShowWindow);
                }
            }
        });
    });
}

fn main() -> anyhow::Result<()> {
    let options = parse_options();
    let instance = instance_path();
    if !options.smoke_test && notify_running_instance(&instance, !options.start_hidden) {
        return Ok(());
    }
    let paths = HubPaths::resolve().context("resolvendo as pastas do hub")?;
    paths.ensure_dirs().context("criando as pastas do hub")?;
    if hub_core::backup::has_pending_restore(&paths) {
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    let restore_result = hub_core::backup::apply_pending_restore(
        &paths,
        hub_core::chrono::Local::now().naive_local(),
    );
    let mut config = Config::load_or_create(&paths.config_file()).context("lendo config.toml")?;
    let autostart = (!options.smoke_test)
        .then(|| hub_core::platform::set_autostart(config.general.start_with_system));
    let translucent_failed = appearance::guard_translucent_start(&paths, &mut config);
    configure_renderer(
        config.general.renderer,
        config.appearance.wallpaper.mode == hub_core::config::WallpaperMode::Translucent,
    );
    let _log_guard = hub_core::logging::init(&paths.logs_dir).context("iniciando os logs")?;
    if let Some(Err(error)) = autostart {
        tracing::warn!(%error, "falha ao sincronizar o início com o sistema");
    }
    if translucent_failed {
        tracing::warn!("a janela translúcida falhou na última abertura e foi desligada");
    }
    match &restore_result {
        Ok(Some(zip)) => tracing::info!(backup = %zip.display(), "backup restaurado"),
        Ok(None) => {}
        Err(error) => tracing::error!(%error, "falha ao restaurar o backup"),
    }
    let db = Database::open(&paths.database_file()).context("abrindo o banco de dados")?;
    let ctx = HubContext::new(
        paths,
        config,
        db,
        hub_core::platform::native(),
        Arc::new(SystemClock),
    );
    if !options.smoke_test {
        listen_for_instances(instance, ctx.events.clone());
    }
    tracing::info!(?options, "hub iniciando");
    app::run(ctx, options).context("executando a interface")?;
    tracing::info!("hub encerrado");
    Ok(())
}
