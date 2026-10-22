pub mod mpv;
mod view;

use std::time::Duration;

use futures::channel::mpsc;
use hub_core::HubContext;
use hub_core::platform::{MediaCommand, MediaInfo, ipc};
use hub_core::player::{self, Favorite};
use hub_ui_kit::Section;
use hub_ui_kit::iced::{Element, Subscription, Task, time};

pub use mpv::{Command, Entry, MpvEvent, MpvProcess, Property, Tools};

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
    status: Status,
    seek_preview: Option<f64>,
    favorites: Vec<Favorite>,
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
            status: Status {
                volume: DEFAULT_VOLUME,
                ..Status::default()
            },
            seek_preview: None,
            favorites: Vec::new(),
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

    fn play(&mut self, url: String) {
        let url = url.trim().to_string();
        if !player::is_supported_link(&url) {
            self.error = Some("Cole um link http ou https do YouTube.".to_string());
            return;
        }
        if let Err(error) = self.start() {
            self.error = Some(error);
            return;
        }
        self.error = None;
        self.current_url = Some(url.clone());
        self.status.title = None;
        self.send(Command::LoadFile(url.clone()));
        let now = self.ctx.clock.now();
        if let Err(error) = player::touch_favorite(self.ctx.db().conn(), &url, now) {
            tracing::warn!(%error, "falha ao salvar favorita");
        }
        self.reload_favorites();
        self.url.clear();
    }

    fn stop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.unbounded_send(Command::Quit);
        }
        self.process = None;
        self.ipc_path = None;
        self.queued.clear();
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
                    && !title.starts_with("http")
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
                self.play(url);
            }
            Message::Play(url) => {
                self.expanded = true;
                self.play(url);
            }
            Message::Mpv(event) => match event {
                MpvEvent::Connected(sender) => {
                    for command in self.queued.drain(..) {
                        let _ = sender.unbounded_send(command);
                    }
                    self.sender = Some(sender);
                }
                MpvEvent::Property(property) => self.apply(property),
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
            Message::Next => self.send(Command::Next),
            Message::Previous => self.send(Command::Previous),
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
            Message::PlayIndex(index) => self.send(Command::PlayIndex(index)),
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
