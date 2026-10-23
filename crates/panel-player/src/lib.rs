pub mod mpv;
pub mod playlist;
mod view;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use hub_core::HubContext;
use hub_core::platform::{self, MediaCommand, MediaInfo, ipc};
use hub_core::player::{self, Favorite};
use hub_ui_kit::Section;
use hub_ui_kit::iced::{Element, Subscription, Task, time};

pub use mpv::{Command, Entry, MpvEvent, MpvProcess, Property, Tools};
pub use playlist::{PlaylistEntry, PlaylistInfo};

pub const SYSTEM_REFRESH: Duration = Duration::from_millis(1500);
pub const SYSTEM_SETTLE: Duration = Duration::from_millis(350);
pub const NO_MEDIA: &str = "nenhuma mídia tocando";
pub const DEFAULT_VOLUME: f64 = 70.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub title: Option<String>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub playlist: Vec<Entry>,
    pub playlist_pos: Option<usize>,
    pub volume: f64,
    pub idle: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToolsChecked(Tools),
    UrlChanged(String),
    PlayInput,
    Play(String),
    PlaylistResolved(String, Option<PlaylistInfo>),
    DirectResolved(String, Option<String>),
    Mpv(MpvEvent),
    TogglePause,
    Next,
    Previous,
    VolumeChanged(f64),
    SeekPreview(f64),
    SeekCommit,
    PlayIndex(usize),
    Stop,
    ToggleExpanded,
    FavoriteRemoved(String),
    FavoritesReload,
    UpdateYtdlp,
    YtdlpUpdated(String),
    SystemTick,
    SystemLoaded(Result<Option<MediaInfo>, String>),
    SystemCommand(MediaCommand),
    SystemCommandDone(Result<(), String>),
    ErrorDismissed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Listing {
    key: String,
    video: Option<String>,
}

pub struct Player {
    ctx: HubContext,
    expanded: bool,
    visible: bool,
    tools: Option<Tools>,
    url: String,
    process: Option<MpvProcess>,
    ipc_path: Option<String>,
    sender: Option<mpsc::UnboundedSender<Command>>,
    queued: Vec<Command>,
    current_url: Option<String>,
    listing: Option<Listing>,
    waiting_list: bool,
    queue: Vec<PlaylistEntry>,
    index: usize,
    preloaded: Option<String>,
    resolving: HashSet<String>,
    cache: playlist::DirectCache,
    titles_ok: bool,
    status: Status,
    seek_preview: Option<f64>,
    favorites: Vec<Favorite>,
    favorite_order: Vec<String>,
    system: Option<MediaInfo>,
    updating_ytdlp: bool,
    notice: Option<String>,
    error: Option<String>,
    session: u64,
}

impl Player {
    pub fn new(ctx: HubContext) -> (Self, Task<Message>) {
        let mut player = Self {
            ctx,
            expanded: false,
            visible: true,
            tools: None,
            url: String::new(),
            process: None,
            ipc_path: None,
            sender: None,
            queued: Vec::new(),
            current_url: None,
            listing: None,
            waiting_list: false,
            queue: Vec::new(),
            index: 0,
            preloaded: None,
            resolving: HashSet::new(),
            cache: playlist::DirectCache::default(),
            titles_ok: true,
            status: Status {
                volume: DEFAULT_VOLUME,
                ..Status::default()
            },
            seek_preview: None,
            favorites: Vec::new(),
            favorite_order: Vec::new(),
            system: None,
            updating_ytdlp: false,
            notice: None,
            error: None,
            session: 0,
        };
        player.reload_favorites();
        let task = Task::batch([
            Task::perform(async { mpv::check_tools() }, Message::ToolsChecked),
            Task::done(Message::SystemTick),
        ]);
        (player, task)
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn tools(&self) -> Option<&Tools> {
        self.tools.as_ref()
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn is_active(&self) -> bool {
        self.process.is_some()
    }

    pub fn loading_playlist(&self) -> bool {
        self.waiting_list
    }

    pub fn is_connected(&self) -> bool {
        self.sender.is_some()
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn position(&self) -> f64 {
        self.seek_preview
            .or(self.status.position)
            .unwrap_or_default()
    }

    pub fn favorites(&self) -> &[Favorite] {
        &self.favorites
    }

    pub fn system(&self) -> Option<&MediaInfo> {
        self.system.as_ref()
    }

    pub fn updating_ytdlp(&self) -> bool {
        self.updating_ytdlp
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn os(&self) -> hub_core::platform::Os {
        self.ctx.platform.os()
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn reload_favorites(&mut self) {
        self.favorites = player::favorites(self.ctx.db().conn()).unwrap_or_default();
    }

    fn send(&mut self, command: Command) {
        match &self.sender {
            Some(sender) => {
                if sender.unbounded_send(command.clone()).is_err() {
                    self.queued.push(command);
                }
            }
            None => self.queued.push(command),
        }
    }

    fn start(&mut self) -> Result<(), String> {
        if self.process.is_some() {
            return Ok(());
        }
        let tools = self.tools.clone().unwrap_or_else(mpv::check_tools);
        let Some(program) = tools.mpv.as_ref().and(tools.mpv_path.clone()) else {
            self.tools = Some(tools);
            return Err("O mpv não está instalado.".to_string());
        };
        let ytdlp = tools.ytdlp_path.clone();
        self.tools = Some(tools);
        self.session += 1;
        let path = ipc::ipc_path(&format!("hub-mpv-{}-{}", std::process::id(), self.session));
        let process = MpvProcess::spawn(&program, ytdlp.as_deref(), &path, self.status.volume)
            .map_err(|e| format!("Não foi possível iniciar o mpv: {e}"))?;
        self.process = Some(process);
        self.ipc_path = Some(path);
        Ok(())
    }

    fn cookie_source(&self) -> Option<String> {
        platform::ytdlp_cookie_source(self.ctx.config().player.browser_login)
    }

    fn start_item(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.queue.get(index).cloned() else {
            return Task::none();
        };
        self.index = index;
        self.preloaded = None;
        let url = self
            .cache
            .get(&entry.id, Instant::now())
            .map(str::to_string)
            .unwrap_or_else(|| playlist::watch_url(&entry.id));
        let title = entry.title.clone().filter(|_| self.titles_ok);
        self.status.title = entry.title.clone();
        self.send(Command::Load { url, title });
        self.prefetch_next()
    }

    fn prefetch_next(&mut self) -> Task<Message> {
        if self.preloaded.is_some() {
            return Task::none();
        }
        let Some(next) = self.queue.get(self.index + 1).cloned() else {
            return Task::none();
        };
        if let Some(direct) = self.cache.get(&next.id, Instant::now()).map(str::to_string) {
            self.append_next(&next, direct);
            return Task::none();
        }
        let Some(ytdlp) = self.tools.as_ref().and_then(|t| t.ytdlp_path.clone()) else {
            self.append_next(&next, playlist::watch_url(&next.id));
            return Task::none();
        };
        if !self.resolving.insert(next.id.clone()) {
            return Task::none();
        }
        let id = next.id.clone();
        Task::perform(
            blocking(move || playlist::fetch_direct(&ytdlp, &id)),
            move |direct| Message::DirectResolved(next.id.clone(), direct),
        )
    }

    fn append_next(&mut self, entry: &PlaylistEntry, url: String) {
        let title = entry.title.clone().filter(|_| self.titles_ok);
        self.send(Command::Append { url, title });
        self.preloaded = Some(entry.id.clone());
    }

    fn direct_resolved(&mut self, id: String, direct: Option<String>) {
        self.resolving.remove(&id);
        let url = match direct {
            Some(direct) => {
                self.cache.insert(&id, direct.clone(), Instant::now());
                direct
            }
            None => playlist::watch_url(&id),
        };
        let still_next = self
            .queue
            .get(self.index + 1)
            .is_some_and(|entry| entry.id == id);
        if still_next
            && self.preloaded.is_none()
            && let Some(entry) = self.queue.get(self.index + 1).cloned()
        {
            self.append_next(&entry, url);
        }
    }

    fn advanced(&mut self) -> Task<Message> {
        if self.preloaded.take().is_none() {
            return Task::none();
        }
        self.index += 1;
        self.send(Command::PlaylistRemove(0));
        self.prefetch_next()
    }

    pub fn queue_items(&self) -> Vec<(String, bool)> {
        if !self.queue.is_empty() {
            return self
                .queue
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let label = entry.title.clone().unwrap_or_else(|| entry.id.clone());
                    (label, index == self.index)
                })
                .collect();
        }
        self.status
            .playlist
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                (
                    entry.label().to_string(),
                    self.status.playlist_pos == Some(index),
                )
            })
            .collect()
    }

    fn play(&mut self, url: String) -> Task<Message> {
        let task = self.play_with(url, true);
        self.favorite_order = self.favorites.iter().map(|f| f.url.clone()).collect();
        task
    }

    fn favorite_neighbor(&self, step: isize) -> Option<String> {
        if self.favorite_order.len() < 2 {
            return None;
        }
        let current = self.current_url.as_ref()?;
        let index = self.favorite_order.iter().position(|url| url == current)? as isize;
        let next = index + step;
        (next >= 0 && (next as usize) < self.favorite_order.len())
            .then(|| self.favorite_order[next as usize].clone())
    }

    fn skip(&mut self, step: isize) -> Task<Message> {
        if !self.queue.is_empty() {
            let target = self.index as isize + step;
            if target >= 0 && (target as usize) < self.queue.len() {
                return self.start_item(target as usize);
            }
        }
        match self.favorite_neighbor(step) {
            Some(url) => self.play_with(url, false),
            None => Task::none(),
        }
    }

    fn play_with(&mut self, url: String, touch: bool) -> Task<Message> {
        let url = url.trim().to_string();
        if !player::is_supported_link(&url) {
            self.error = Some("Cole um link http ou https do YouTube.".to_string());
            return Task::none();
        }
        if let Err(error) = self.start() {
            self.error = Some(error);
            return Task::none();
        }
        self.error = None;
        self.current_url = Some(url.clone());
        self.status.title = None;
        self.listing = None;
        self.waiting_list = false;
        self.queue.clear();
        self.index = 0;
        self.preloaded = None;
        self.resolving.clear();
        let plan = playlist::plan(&url);
        let ytdlp = self.tools.as_ref().and_then(|t| t.ytdlp_path.clone());
        let mut task = Task::none();
        match (plan.lookup.clone(), ytdlp) {
            (Some(list), Some(ytdlp)) => {
                if let Some(video) = plan.start_video.clone() {
                    self.queue.push(PlaylistEntry {
                        id: video,
                        title: None,
                    });
                    task = self.start_item(0);
                } else {
                    self.waiting_list = true;
                }
                self.listing = Some(Listing {
                    key: url.clone(),
                    video: plan.start_video.clone(),
                });
                let key = url.clone();
                task = Task::batch([
                    task,
                    Task::perform(
                        {
                            let cookies = self.cookie_source();
                            blocking(move || {
                                playlist::fetch_info(&ytdlp, &list, cookies.as_deref())
                            })
                        },
                        move |info| Message::PlaylistResolved(key.clone(), info),
                    ),
                ]);
            }
            (Some(list), None) => {
                let target = plan.first.clone().unwrap_or(list);
                self.send(Command::Load {
                    url: target,
                    title: None,
                });
            }
            (None, _) => {
                if let Some(first) = plan.first.clone() {
                    self.send(Command::Load {
                        url: first,
                        title: None,
                    });
                }
            }
        }
        if touch {
            let now = self.ctx.clock.now();
            if let Err(error) = player::touch_favorite(self.ctx.db().conn(), &url, now) {
                tracing::warn!(%error, "falha ao salvar favorita");
            }
            self.reload_favorites();
        }
        self.url.clear();
        task
    }

    fn resolve_playlist(&mut self, key: String, info: Option<PlaylistInfo>) -> Task<Message> {
        if let Some(info) = &info
            && let Some(title) = &info.title
        {
            let _ = player::rename_favorite(self.ctx.db().conn(), &key, title);
            self.reload_favorites();
        }
        let Some(listing) = self.listing.take_if(|listing| listing.key == key) else {
            return Task::none();
        };
        self.waiting_list = false;
        let entries = info.map(|info| info.entries).unwrap_or_default();
        if entries.is_empty() {
            if self.queue.is_empty() {
                self.error = Some(
                    "Não foi possível carregar a playlist. Se ela for privada, o YouTube só mostra as faixas para quem está logado.".to_string(),
                );
            } else {
                self.notice = Some(
                    "Não foi possível ler as outras faixas da playlist (ela pode ser privada). Tocando só este vídeo.".to_string(),
                );
            }
            return Task::none();
        }
        let position = listing
            .video
            .as_deref()
            .and_then(|video| entries.iter().position(|entry| entry.id == video));
        match position {
            Some(position) => {
                self.queue = entries;
                self.index = position;
                if let Some(entry) = self.queue.get(position)
                    && self.status.title.is_none()
                {
                    self.status.title = entry.title.clone();
                }
                self.prefetch_next()
            }
            None if self.queue.is_empty() => {
                self.queue = entries;
                self.start_item(0)
            }
            None => {
                self.queue.extend(entries);
                self.prefetch_next()
            }
        }
    }

    fn command_failed(&mut self, command: Command, error: &str) {
        if let Some(plain) = command.without_title() {
            self.titles_ok = false;
            self.send(plain);
        } else {
            tracing::debug!(?command, %error, "comando do mpv falhou");
        }
    }

    fn stop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.unbounded_send(Command::Quit);
        }
        self.process = None;
        self.ipc_path = None;
        self.queued.clear();
        self.listing = None;
        self.waiting_list = false;
        self.queue.clear();
        self.index = 0;
        self.preloaded = None;
        self.resolving.clear();
        self.current_url = None;
        let volume = self.status.volume;
        self.status = Status {
            volume,
            ..Status::default()
        };
        self.seek_preview = None;
    }

    fn apply(&mut self, property: Property) {
        match property {
            Property::Title(title) => {
                if let (Some(url), Some(title)) = (&self.current_url, &title)
                    && !player::is_placeholder_title(title)
                    && !player::is_playlist(url)
                {
                    let _ = player::name_favorite(self.ctx.db().conn(), url, title);
                    self.reload_favorites();
                }
                self.status.title = title;
            }
            Property::Position(position) => self.status.position = position,
            Property::Duration(duration) => self.status.duration = duration,
            Property::Paused(paused) => self.status.paused = paused,
            Property::PlaylistPos(pos) => self.status.playlist_pos = pos,
            Property::Playlist(list) => self.status.playlist = list,
            Property::Volume(volume) => self.status.volume = volume,
            Property::Idle(idle) => self.status.idle = idle,
        }
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => value,
        Err(error) => std::panic::resume_unwind(error.into_panic()),
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Section for Player {
    type Message = Message;

    fn title(&self) -> &str {
        "Player"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToolsChecked(tools) => self.tools = Some(tools),
            Message::UrlChanged(url) => self.url = url,
            Message::PlayInput => {
                let url = self.url.clone();
                return self.play(url);
            }
            Message::Play(url) => {
                self.expanded = true;
                return self.play(url);
            }
            Message::PlaylistResolved(url, info) => return self.resolve_playlist(url, info),
            Message::DirectResolved(id, direct) => self.direct_resolved(id, direct),
            Message::Mpv(event) => match event {
                MpvEvent::Connected(sender) => {
                    for command in self.queued.drain(..) {
                        let _ = sender.unbounded_send(command);
                    }
                    self.sender = Some(sender);
                }
                MpvEvent::Property(property) => {
                    let advanced = matches!(property, Property::PlaylistPos(Some(pos)) if pos >= 1)
                        && self.preloaded.is_some();
                    let finished = matches!(property, Property::Idle(true))
                        && !self.status.idle
                        && self.status.title.is_some()
                        && self.index + 1 >= self.queue.len();
                    self.apply(property);
                    if advanced {
                        return self.advanced();
                    }
                    if finished
                        && !self.waiting_list
                        && let Some(next) = self.favorite_neighbor(1)
                    {
                        return self.play_with(next, false);
                    }
                }
                MpvEvent::Failed(command, error) => self.command_failed(command, &error),
                MpvEvent::Disconnected(reason) => {
                    let was_active = self.process.is_some();
                    self.sender = None;
                    self.stop();
                    if was_active {
                        self.notice = Some(reason);
                    }
                }
            },
            Message::TogglePause => self.send(Command::TogglePause),
            Message::Next => return self.skip(1),
            Message::Previous => return self.skip(-1),
            Message::VolumeChanged(volume) => {
                self.status.volume = volume;
                if self.is_active() {
                    self.send(Command::SetVolume(volume));
                }
            }
            Message::SeekPreview(position) => self.seek_preview = Some(position),
            Message::SeekCommit => {
                if let Some(position) = self.seek_preview.take() {
                    self.status.position = Some(position);
                    self.send(Command::Seek(position));
                }
            }
            Message::PlayIndex(index) => {
                if !self.queue.is_empty() {
                    return self.start_item(index);
                }
                self.send(Command::PlayIndex(index));
            }
            Message::Stop => self.stop(),
            Message::ToggleExpanded => self.expanded = !self.expanded,
            Message::FavoriteRemoved(url) => {
                let _ = player::remove_favorite(self.ctx.db().conn(), &url);
                self.reload_favorites();
            }
            Message::FavoritesReload => self.reload_favorites(),
            Message::UpdateYtdlp => {
                self.updating_ytdlp = true;
                return Task::perform(async { mpv::update_ytdlp() }, Message::YtdlpUpdated);
            }
            Message::YtdlpUpdated(result) => {
                self.updating_ytdlp = false;
                self.notice = Some(result);
                return Task::perform(async { mpv::check_tools() }, Message::ToolsChecked);
            }
            Message::SystemTick => {
                if self.is_active() || !self.visible {
                    return Task::none();
                }
                let platform = self.ctx.platform.clone();
                return Task::perform(
                    blocking(move || {
                        platform
                            .media_session()
                            .and_then(|session| session.current())
                            .map_err(|e| e.to_string())
                    }),
                    Message::SystemLoaded,
                );
            }
            Message::SystemLoaded(result) => self.system = result.ok().flatten(),
            Message::SystemCommand(command) => {
                if command == MediaCommand::PlayPause
                    && let Some(media) = &mut self.system
                {
                    media.playing = !media.playing;
                }
                let platform = self.ctx.platform.clone();
                return Task::perform(
                    blocking(move || {
                        platform
                            .media_session()
                            .and_then(|session| session.send(command))
                            .map_err(|e| e.to_string())
                    }),
                    Message::SystemCommandDone,
                );
            }
            Message::SystemCommandDone(result) => {
                if let Err(error) = result
                    && !error.contains(NO_MEDIA)
                {
                    self.notice = Some(error);
                }
                return Task::perform(async { tokio::time::sleep(SYSTEM_SETTLE).await }, |()| {
                    Message::SystemTick
                });
            }
            Message::ErrorDismissed => {
                self.error = None;
                self.notice = None;
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::new();
        if let Some(path) = &self.ipc_path {
            subscriptions.push(
                Subscription::run_with(path.clone(), |path| mpv::connect(path.clone()))
                    .map(Message::Mpv),
            );
        }
        if self.visible && !self.is_active() {
            subscriptions.push(time::every(SYSTEM_REFRESH).map(|_| Message::SystemTick));
        }
        Subscription::batch(subscriptions)
    }
}

#[cfg(test)]
mod tests;
