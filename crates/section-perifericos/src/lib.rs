mod view;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use hub_core::HubContext;
use hub_core::leds::{self, Rgb, Snapshot, Target};
use hub_ui_kit::Section;
use hub_ui_kit::color_picker::{self, ColorPicker, PickerMessage};
use hub_ui_kit::iced::{Color, Element, Subscription, Task};

const RETRY_DELAY: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubTab {
    #[default]
    Leds,
}

impl SubTab {
    pub const ALL: [SubTab; 1] = [SubTab::Leds];

    pub fn label(self) -> &'static str {
        match self {
            SubTab::Leds => "LEDs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    All,
    Device(u32),
}

#[derive(Debug, Clone)]
pub enum Message {
    SubTab(SubTab),
    Retry,
    OpenOpenRgb,
    Loaded(Result<Snapshot, String>),
    Focus(Focus),
    Swatch(Focus, Color),
    Picker(PickerMessage),
    Painted(Result<usize, String>),
    Mode(u32, usize),
    Speed(u32, u32),
    SpeedReleased(u32),
    Profile(String),
    ProfileApplied(Result<(), String>),
    GameProfile(String),
    NormalProfile(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Loading,
    Ready(Snapshot),
    Failed(String),
}

pub struct Peripherals {
    ctx: HubContext,
    sub_tab: SubTab,
    status: Status,
    focus: Focus,
    picker: ColorPicker,
    colors: HashMap<u32, Color>,
    all_color: Color,
    speeds: HashMap<u32, u32>,
    in_flight: bool,
    pending: Option<(Target, Rgb)>,
    notice: Option<String>,
    openrgb: Option<PathBuf>,
}

fn rgb(color: Color) -> Rgb {
    let [r, g, b] = [color.r, color.g, color.b].map(|c| (c * 255.0).round() as u8);
    [r, g, b]
}

impl Peripherals {
    pub fn new(ctx: HubContext) -> Self {
        Self {
            ctx,
            sub_tab: SubTab::Leds,
            status: Status::Loading,
            focus: Focus::All,
            picker: ColorPicker::new(color_picker::SWATCHES[0]),
            colors: HashMap::new(),
            all_color: color_picker::SWATCHES[0],
            speeds: HashMap::new(),
            in_flight: false,
            pending: None,
            notice: None,
            openrgb: None,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn sub_tab(&self) -> SubTab {
        self.sub_tab
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn leds_config(&self) -> hub_core::config::LedsConfig {
        self.ctx.config().leds
    }

    fn port(&self) -> u16 {
        self.ctx.config().leds.port
    }

    fn load(&mut self, delay: Duration) -> Task<Message> {
        self.status = Status::Loading;
        self.openrgb = leds::find_openrgb();
        let port = self.port();
        Task::perform(
            async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                leds::snapshot(port).await.map_err(|e| e.to_string())
            },
            Message::Loaded,
        )
    }

    fn paint(&mut self, target: Target, color: Rgb) -> Task<Message> {
        if self.in_flight {
            self.pending = Some((target, color));
            return Task::none();
        }
        self.in_flight = true;
        let port = self.port();
        Task::perform(
            async move {
                leds::paint(port, target, color)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::Painted,
        )
    }

    fn color_of(&self, focus: Focus) -> Color {
        match focus {
            Focus::All => self.all_color,
            Focus::Device(index) => self.colors.get(&index).copied().unwrap_or(self.all_color),
        }
    }

    fn choose_color(&mut self, focus: Focus, color: Color) -> Task<Message> {
        let target = match focus {
            Focus::All => {
                self.all_color = color;
                self.colors.clear();
                Target::All
            }
            Focus::Device(index) => {
                self.colors.insert(index, color);
                Target::Index(index)
            }
        };
        self.paint(target, rgb(color))
    }

    pub fn device_speed(&self, index: u32) -> Option<u32> {
        self.speeds.get(&index).copied()
    }

    pub fn device_color(&self, index: u32) -> Color {
        self.color_of(Focus::Device(index))
    }

    pub fn all_color(&self) -> Color {
        self.all_color
    }

    pub fn picker(&self) -> &ColorPicker {
        &self.picker
    }

    pub fn openrgb(&self) -> Option<&PathBuf> {
        self.openrgb.as_ref()
    }

    fn save_profile_choice(&mut self, game: bool, name: String) {
        let mut config = self.ctx.config();
        let name = if name == view::NONE_LABEL {
            String::new()
        } else {
            name
        };
        if game {
            config.leds.game_profile = name;
        } else {
            config.leds.normal_profile = name;
        }
        if let Err(error) = config.save(&self.ctx.paths.config_file()) {
            tracing::warn!(%error, "falha ao salvar a configuração dos LEDs");
        }
        *self.ctx.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
    }

    fn set_active_mode(&mut self, device: u32, mode: usize) {
        if let Status::Ready(snapshot) = &mut self.status
            && let Some(found) = snapshot.devices.iter_mut().find(|d| d.index == device)
        {
            found.active_mode = mode;
        }
    }
}

impl Section for Peripherals {
    type Message = Message;

    fn title(&self) -> &str {
        "Periféricos"
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SubTab(tab) => self.sub_tab = tab,
            Message::Retry => return self.load(Duration::ZERO),
            Message::OpenOpenRgb => {
                if let Some(path) = &self.openrgb {
                    if let Err(error) = leds::start_openrgb(path) {
                        tracing::warn!(%error, "falha ao abrir o OpenRGB");
                        self.notice = Some("Não foi possível abrir o OpenRGB".to_string());
                        return Task::none();
                    }
                    return self.load(RETRY_DELAY);
                }
            }
            Message::Loaded(result) => match result {
                Ok(snapshot) => {
                    self.notice = None;
                    for device in &snapshot.devices {
                        if let Some(mode) = device.active() {
                            self.speeds.entry(device.index).or_insert(mode.speed);
                        }
                    }
                    self.status = Status::Ready(snapshot);
                }
                Err(error) => self.status = Status::Failed(error),
            },
            Message::Focus(focus) => {
                self.focus = focus;
                self.picker.set_color(self.color_of(focus));
            }
            Message::Swatch(focus, color) => {
                self.picker.set_color(color);
                return self.choose_color(focus, color);
            }
            Message::Picker(message) => {
                if let Some(color) = self.picker.update(message) {
                    return self.choose_color(self.focus, color);
                }
            }
            Message::Painted(result) => {
                self.in_flight = false;
                self.notice = result.err();
                if let Some((target, color)) = self.pending.take() {
                    return self.paint(target, color);
                }
            }
            Message::Mode(device, mode) => {
                self.set_active_mode(device, mode);
                let port = self.port();
                let color = self.colors.get(&device).copied().map(rgb);
                let speed = self.speeds.get(&device).copied();
                return Task::perform(
                    async move {
                        leds::set_mode(port, device, mode, speed, color)
                            .await
                            .map(|_| 1)
                            .map_err(|e| e.to_string())
                    },
                    Message::Painted,
                );
            }
            Message::Speed(device, speed) => {
                self.speeds.insert(device, speed);
            }
            Message::SpeedReleased(device) => {
                let active = match &self.status {
                    Status::Ready(snapshot) => snapshot
                        .devices
                        .iter()
                        .find(|d| d.index == device)
                        .map(|d| d.active_mode),
                    _ => None,
                };
                if let Some(mode) = active {
                    return self.update(Message::Mode(device, mode));
                }
            }
            Message::Profile(name) => {
                let port = self.port();
                return Task::perform(
                    async move {
                        leds::apply_profile(port, &name)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::ProfileApplied,
                );
            }
            Message::ProfileApplied(result) => {
                self.notice = result.err();
                return self.load(Duration::ZERO);
            }
            Message::GameProfile(name) => self.save_profile_choice(true, name),
            Message::NormalProfile(name) => self.save_profile_choice(false, name),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        view::view(self)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::none()
    }
}
