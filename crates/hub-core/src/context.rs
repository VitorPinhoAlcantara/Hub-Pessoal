use std::sync::{Arc, Mutex, MutexGuard};

use crate::clock::Clock;
use crate::events::EventBus;
use crate::paths::HubPaths;
use crate::platform::Platform;
use crate::{Config, Database};

#[derive(Clone)]
pub struct HubContext {
    pub paths: HubPaths,
    pub config: Arc<Mutex<Config>>,
    pub db: Arc<Mutex<Database>>,
    pub platform: Arc<dyn Platform>,
    pub clock: Arc<dyn Clock>,
    pub events: EventBus,
}

impl HubContext {
    pub fn new(
        paths: HubPaths,
        config: Config,
        db: Database,
        platform: Arc<dyn Platform>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            paths,
            config: Arc::new(Mutex::new(config)),
            db: Arc::new(Mutex::new(db)),
            platform,
            clock,
            events: EventBus::new(),
        }
    }

    pub fn db(&self) -> MutexGuard<'_, Database> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn config(&self) -> Config {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
