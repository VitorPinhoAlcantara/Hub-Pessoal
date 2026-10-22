use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::Config;
use crate::context::HubContext;
use crate::events::HubEvent;
use crate::paths::HubPaths;

pub struct ConfigWatcher {
    _inner: RecommendedWatcher,
}

pub fn classify(paths: &HubPaths, changed: &Path) -> Option<HubEvent> {
    if changed == paths.config_file() {
        Some(HubEvent::ConfigChanged)
    } else if changed == paths.shortcuts_file() {
        Some(HubEvent::ShortcutsChanged)
    } else if changed.starts_with(&paths.scripts_dir) {
        Some(HubEvent::ScriptsChanged)
    } else {
        None
    }
}

pub fn reload_config(ctx: &HubContext) -> bool {
    match Config::load(&ctx.paths.config_file()) {
        Ok(config) => {
            *ctx.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
            true
        }
        Err(error) => {
            tracing::warn!(%error, "config.toml inválido, mantendo a última versão válida");
            false
        }
    }
}

pub fn spawn(ctx: HubContext) -> notify::Result<ConfigWatcher> {
    let handler_ctx = ctx.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        let mut published: Vec<HubEvent> = Vec::new();
        for path in &event.paths {
            let Some(kind) = classify(&handler_ctx.paths, path) else {
                continue;
            };
            if published.contains(&kind) {
                continue;
            }
            if kind == HubEvent::ConfigChanged && !reload_config(&handler_ctx) {
                continue;
            }
            handler_ctx.events.publish(kind.clone());
            published.push(kind);
        }
    })?;
    let config_dir: PathBuf = ctx.paths.config_dir.clone();
    watcher.watch(&config_dir, RecursiveMode::Recursive)?;
    Ok(ConfigWatcher { _inner: watcher })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::clock::FakeClock;
    use crate::platform::FakePlatform;
    use crate::{Database, config::Renderer};

    #[test]
    fn classify_maps_files_to_events() {
        let paths = HubPaths::from_home("base");
        assert_eq!(
            classify(&paths, &paths.config_file()),
            Some(HubEvent::ConfigChanged)
        );
        assert_eq!(
            classify(&paths, &paths.shortcuts_file()),
            Some(HubEvent::ShortcutsChanged)
        );
        assert_eq!(
            classify(&paths, &paths.scripts_dir.join("x.toml")),
            Some(HubEvent::ScriptsChanged)
        );
        assert_eq!(classify(&paths, &paths.data_dir.join("hub.db")), None);
    }

    fn wait_for(
        receiver: &mut tokio::sync::broadcast::Receiver<HubEvent>,
        wanted: HubEvent,
    ) -> bool {
        for _ in 0..50 {
            while let Ok(event) = receiver.try_recv() {
                if event == wanted {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    #[test]
    fn file_changes_publish_events_and_reload_config() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path());
        paths.ensure_dirs().unwrap();
        Config::default().save(&paths.config_file()).unwrap();
        let ctx = HubContext::new(
            paths.clone(),
            Config::default(),
            Database::open_in_memory().unwrap(),
            Arc::new(FakePlatform::default()),
            Arc::new(FakeClock::at("2026-10-21 10:00")),
        );
        let mut events = ctx.events.subscribe();
        let _watcher = spawn(ctx.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(200));

        std::fs::write(paths.shortcuts_file(), "").unwrap();
        assert!(wait_for(&mut events, HubEvent::ShortcutsChanged));

        std::fs::write(paths.scripts_dir.join("a.toml"), "nome = \"a\"").unwrap();
        assert!(wait_for(&mut events, HubEvent::ScriptsChanged));

        std::fs::write(
            paths.config_file(),
            "[geral]\nrenderizador = \"software\"\n",
        )
        .unwrap();
        assert!(wait_for(&mut events, HubEvent::ConfigChanged));
        assert_eq!(ctx.config().general.renderer, Renderer::Software);

        std::fs::write(paths.config_file(), "[geral\n").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(ctx.config().general.renderer, Renderer::Software);
    }
}
