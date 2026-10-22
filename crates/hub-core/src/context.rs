use std::sync::{Arc, Mutex, MutexGuard};

use chrono::NaiveDateTime;

use crate::clock::Clock;
use crate::confirm::ConfirmationBroker;
use crate::events::EventBus;
use crate::paths::HubPaths;
use crate::platform::Platform;
use crate::scripts::{Script, ScriptStore};
use crate::shortcuts::{Loaded, ShortcutStore};
use crate::{Config, Database};

#[derive(Clone)]
pub struct HubContext {
    pub paths: HubPaths,
    pub config: Arc<Mutex<Config>>,
    pub db: Arc<Mutex<Database>>,
    pub platform: Arc<dyn Platform>,
    pub clock: Arc<dyn Clock>,
    pub events: EventBus,
    pub script_store: Arc<Mutex<ScriptStore>>,
    pub shortcut_store: Arc<Mutex<ShortcutStore>>,
    pub scheduler_mark: Arc<Mutex<NaiveDateTime>>,
    pub confirmations: ConfirmationBroker,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl HubContext {
    pub fn new(
        paths: HubPaths,
        config: Config,
        db: Database,
        platform: Arc<dyn Platform>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let now = clock.now();
        Self {
            paths,
            config: Arc::new(Mutex::new(config)),
            db: Arc::new(Mutex::new(db)),
            platform,
            clock,
            events: EventBus::new(),
            script_store: Arc::new(Mutex::new(ScriptStore::default())),
            shortcut_store: Arc::new(Mutex::new(ShortcutStore::default())),
            scheduler_mark: Arc::new(Mutex::new(now)),
            confirmations: ConfirmationBroker::default(),
        }
    }

    pub fn db(&self) -> MutexGuard<'_, Database> {
        lock(&self.db)
    }

    pub fn config(&self) -> Config {
        lock(&self.config).clone()
    }

    pub fn scripts(&self) -> Vec<Script> {
        lock(&self.script_store).list(&self.paths.scripts_dir, self.platform.os())
    }

    pub fn script(&self, id: &str) -> crate::scripts::Result<Script> {
        lock(&self.script_store).get(&self.paths.scripts_dir, self.platform.os(), id)
    }

    pub fn shortcuts(&self) -> Loaded {
        lock(&self.shortcut_store).load(&self.paths.shortcuts_file())
    }

    pub fn shortcut_store(&self) -> MutexGuard<'_, ShortcutStore> {
        lock(&self.shortcut_store)
    }

    pub fn take_scheduler_mark(&self, now: NaiveDateTime) -> NaiveDateTime {
        std::mem::replace(&mut *lock(&self.scheduler_mark), now)
    }
}
