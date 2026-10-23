use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hub_core::HubContext;
use hub_core::config::{AppearanceConfig, CustomColors, ThemeChoice, WallpaperMode, WindowBlur};
use hub_core::{platform, wallpaper};
use hub_ui_kit::color_picker::{self, ColorPicker, PickerMessage};
use hub_ui_kit::tokens::*;
use hub_ui_kit::widgets::*;
use hub_ui_kit::{dialog, font};
use iced::widget::{Column, Row, button, column, container, row, slider, space, text};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};

use crate::app::Message as AppMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSlot {
    #[default]
    Accent,
    Secondary,
    Background,
    Surface,
    Text,
}

impl ColorSlot {
    const ALL: [ColorSlot; 5] = [
        ColorSlot::Accent,
        ColorSlot::Secondary,
        ColorSlot::Background,
        ColorSlot::Surface,
        ColorSlot::Text,
    ];

    fn label(self) -> &'static str {
        match self {
            ColorSlot::Accent => "Principal",
            ColorSlot::Secondary => "Secundária",
            ColorSlot::Background => "Fundo",
            ColorSlot::Surface => "Painéis",
            ColorSlot::Text => "Texto",
        }
    }

    fn get(self, palette: &Palette) -> Color {
        match self {
            ColorSlot::Accent => palette.accent,
            ColorSlot::Secondary => palette.secondary,
            ColorSlot::Background => palette.background,
            ColorSlot::Surface => palette.surface,
            ColorSlot::Text => palette.text,
        }
    }

    fn set(self, palette: &mut Palette, color: Color) {
        match self {
            ColorSlot::Accent => palette.accent = color,
            ColorSlot::Secondary => palette.secondary = color,
            ColorSlot::Background => palette.background = color,
            ColorSlot::Surface => palette.surface = color,
            ColorSlot::Text => palette.text = color,
        }
    }

    fn swatches(self) -> Vec<Color> {
        match self {
            ColorSlot::Accent | ColorSlot::Secondary => color_picker::SWATCHES.to_vec(),
            ColorSlot::Background => vec![
                hex(0x0B0C0E),
                hex(0x111214),
                hex(0x16181D),
                hex(0x1B1626),
                hex(0x0F1A17),
                hex(0xE9EAEE),
                hex(0xF4F1EA),
                hex(0xFFFFFF),
            ],
            ColorSlot::Surface => vec![
                hex(0x16171A),
                hex(0x1A1B1E),
                hex(0x202228),
                hex(0x241E30),
                hex(0x16231F),
                hex(0xFFFFFF),
                hex(0xFBF9F4),
                hex(0xF1F2F5),
            ],
            ColorSlot::Text => vec![
                hex(0xFFFFFF),
                hex(0xE6E6E7),
                hex(0xC9CBD1),
                hex(0x1C1D21),
                hex(0x2B2D33),
                hex(0x000000),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Theme(ThemeChoice),
    Slot(ColorSlot),
    Swatch(Color),
    Picker(PickerMessage),
    ToggleAdvanced,
    ResetCustom,
    FontPreset(&'static str),
    FontInput(String),
    FontApply,
    FontPick(String),
    FontsLoaded(Vec<String>),
    Restart,
    WallpaperMode(WallpaperMode),
    WindowBlur(WindowBlur),
    PickImage,
    ImageChosen(Option<PathBuf>),
    Blur(u32),
    BlurReleased,
    Dim(f32),
    DimReleased,
}

pub enum Outcome {
    None,
    Restart,
    Wallpaper,
    WindowBlur,
    BlurPreview(u32),
    Task(Task<Message>),
}

static TRANSLUCENT_SESSION: AtomicBool = AtomicBool::new(false);
static TRANSLUCENT_FAILED: AtomicBool = AtomicBool::new(false);
const TRANSLUCENT_MARKER: &str = "translucido-abrindo";

pub fn guard_translucent_start(
    paths: &hub_core::HubPaths,
    config: &mut hub_core::Config,
) -> bool {
    let marker = paths.data_dir.join(TRANSLUCENT_MARKER);
    let failed = marker.exists() && config.appearance.wallpaper.mode == WallpaperMode::Translucent;
    if failed {
        config.appearance.wallpaper.mode = WallpaperMode::None;
        let _ = config.save(&paths.config_file());
        TRANSLUCENT_FAILED.store(true, Ordering::Relaxed);
    }
    let _ = std::fs::remove_file(&marker);
    if config.appearance.wallpaper.mode == WallpaperMode::Translucent {
        let _ = std::fs::write(&marker, "");
    }
    failed
}

pub fn confirm_translucent_start(paths: &hub_core::HubPaths) {
    let _ = std::fs::remove_file(paths.data_dir.join(TRANSLUCENT_MARKER));
}

pub fn translucent_failed() -> bool {
    TRANSLUCENT_FAILED.load(Ordering::Relaxed)
}

pub fn start_session(config: &AppearanceConfig) -> bool {
    let translucent = config.wallpaper.mode == WallpaperMode::Translucent;
    TRANSLUCENT_SESSION.store(translucent, Ordering::Relaxed);
    translucent
}

pub fn translucent_session() -> bool {
    TRANSLUCENT_SESSION.load(Ordering::Relaxed)
}

fn effective_translucency(config: &AppearanceConfig) -> Option<f32> {
    let dim = config.wallpaper.clamped_dim();
    match config.wallpaper.mode {
        WallpaperMode::None => None,
        WallpaperMode::Image | WallpaperMode::System => Some(dim),
        WallpaperMode::Translucent => translucent_session().then_some(dim),
    }
}

#[derive(Debug, Clone)]
pub struct WallpaperFrame {
    pub key: String,
    pub blur: u32,
    pub base: Arc<wallpaper::RgbaImage>,
    pub handle: iced::widget::image::Handle,
}

fn frame(key: String, base: Arc<wallpaper::RgbaImage>, blur: u32) -> WallpaperFrame {
    let soft = wallpaper::blurred(&base, blur);
    let (width, height) = soft.dimensions();
    let handle = iced::widget::image::Handle::from_rgba(width, height, soft.into_raw());
    WallpaperFrame {
        key,
        blur,
        base,
        handle,
    }
}

pub async fn blur_wallpaper(current: WallpaperFrame, blur: u32) -> Option<WallpaperFrame> {
    tokio::task::spawn_blocking(move || frame(current.key, current.base, blur))
        .await
        .ok()
}

pub async fn load_wallpaper(
    ctx: HubContext,
    current: Option<WallpaperFrame>,
) -> Result<Option<WallpaperFrame>, String> {
    let config = ctx.config().appearance.wallpaper;
    let source = match config.mode {
        WallpaperMode::Image if !config.image.trim().is_empty() => {
            PathBuf::from(config.image.trim())
        }
        WallpaperMode::System => match platform::desktop_wallpaper() {
            Some(path) => path,
            None => return Err("papel de parede do Windows não encontrado".to_string()),
        },
        _ => return Ok(None),
    };
    let blur = config.clamped_blur();
    tokio::task::spawn_blocking(move || {
        let key = wallpaper::source_key(&source).map_err(|error| error.to_string())?;
        if let Some(current) = current.filter(|frame| frame.key == key) {
            if current.blur == blur {
                return Ok(Some(current));
            }
            return Ok(Some(frame(key, current.base, blur)));
        }
        let base = wallpaper::load_base(&ctx.paths, &source).map_err(|error| error.to_string())?;
        Ok(Some(frame(key, Arc::new(base), blur)))
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn acrylic_tint() -> (u8, u8, u8, u8) {
    let background = palette().background;
    (
        (background.r * 255.0) as u8,
        (background.g * 255.0) as u8,
        (background.b * 255.0) as u8,
        24,
    )
}

pub struct Personalization {
    slot: ColorSlot,
    picker: ColorPicker,
    advanced: bool,
    font_input: String,
    installed: Option<Vec<String>>,
    loading_fonts: bool,
    font_saved: String,
    font_running: String,
    blur: Option<u32>,
    dim: Option<f32>,
    status: Option<String>,
}

pub fn palette_for(config: &AppearanceConfig) -> Palette {
    match config.theme {
        ThemeChoice::Dark => Palette::DARK,
        ThemeChoice::Light => Palette::LIGHT,
        ThemeChoice::Custom => custom_palette(&config.colors),
    }
}

fn custom_palette(colors: &CustomColors) -> Palette {
    let base = Palette::DARK;
    let parse = |value: &str, fallback: Color| color_picker::parse_hex(value).unwrap_or(fallback);
    let background = parse(&colors.background, base.background);
    let light = luminance(background) > 0.5;
    let semantic = if light { Palette::LIGHT } else { Palette::DARK };
    Palette {
        background,
        surface: parse(&colors.surface, base.surface),
        text: parse(&colors.text, base.text),
        accent: parse(&colors.accent, base.accent),
        secondary: parse(&colors.secondary, base.secondary),
        danger: semantic.danger,
        success: semantic.success,
        warning: semantic.warning,
    }
}

fn colors_from(palette: &Palette) -> CustomColors {
    CustomColors {
        background: color_picker::to_hex(palette.background),
        surface: color_picker::to_hex(palette.surface),
        text: color_picker::to_hex(palette.text),
        accent: color_picker::to_hex(palette.accent),
        secondary: color_picker::to_hex(palette.secondary),
    }
}

pub fn apply(config: &AppearanceConfig) {
    set_palette(palette_for(config));
    set_translucency(effective_translucency(config));
}

impl Personalization {
    pub fn new(ctx: &HubContext) -> Self {
        let appearance = ctx.config().appearance;
        let palette = palette_for(&appearance);
        let font = display_font(&appearance.font);
        Self {
            slot: ColorSlot::Accent,
            picker: ColorPicker::new(palette.accent),
            advanced: false,
            font_input: if font::CHOICES.contains(&font.as_str()) {
                String::new()
            } else {
                font.clone()
            },
            installed: None,
            loading_fonts: false,
            font_saved: font.clone(),
            font_running: font,
            blur: None,
            dim: None,
            status: None,
        }
    }

    pub fn update(&mut self, ctx: &HubContext, message: Message) -> Outcome {
        match message {
            Message::WallpaperMode(mode) => {
                self.save(ctx, |appearance| appearance.wallpaper.mode = mode);
                if mode == WallpaperMode::Image
                    && ctx.config().appearance.wallpaper.image.trim().is_empty()
                {
                    return Outcome::Task(Task::done(Message::PickImage));
                }
                return Outcome::Wallpaper;
            }
            Message::WindowBlur(blur) => {
                self.save(ctx, |appearance| appearance.wallpaper.window_blur = blur);
                return Outcome::WindowBlur;
            }
            Message::PickImage => {
                return Outcome::Task(Task::perform(
                    dialog::pick_file("Escolher imagem de fundo"),
                    Message::ImageChosen,
                ));
            }
            Message::ImageChosen(None) => {}
            Message::ImageChosen(Some(path)) => {
                let value = path.display().to_string();
                self.save(ctx, |appearance| {
                    appearance.wallpaper.mode = WallpaperMode::Image;
                    appearance.wallpaper.image = value;
                });
                return Outcome::Wallpaper;
            }
            Message::Blur(value) => {
                self.blur = Some(value);
                return Outcome::BlurPreview(value);
            }
            Message::BlurReleased => {
                if let Some(value) = self.blur.take() {
                    self.save(ctx, |appearance| appearance.wallpaper.blur = value);
                    return Outcome::Wallpaper;
                }
            }
            Message::Dim(value) => {
                self.dim = Some(value);
                let mut appearance = ctx.config().appearance;
                appearance.wallpaper.dim = value;
                set_translucency(effective_translucency(&appearance));
            }
            Message::DimReleased => {
                if let Some(value) = self.dim.take() {
                    self.save(ctx, |appearance| appearance.wallpaper.dim = value);
                }
            }

            Message::Theme(choice) => {
                self.save(ctx, |appearance| appearance.theme = choice);
                self.sync_picker(ctx);
            }
            Message::Slot(slot) => {
                self.slot = slot;
                self.sync_picker(ctx);
            }
            Message::Swatch(color) => {
                self.set_color(ctx, color);
                self.picker.set_color(color);
            }
            Message::Picker(message) => {
                if let Some(color) = self.picker.update(message) {
                    self.set_color(ctx, color);
                }
            }
            Message::ToggleAdvanced => self.advanced = !self.advanced,
            Message::ResetCustom => {
                let base = if palette().is_light() {
                    Palette::LIGHT
                } else {
                    Palette::DARK
                };
                self.save(ctx, |appearance| {
                    appearance.theme = ThemeChoice::Custom;
                    appearance.colors = colors_from(&base);
                });
                self.sync_picker(ctx);
            }
            Message::FontPreset(name) => {
                self.font_input.clear();
                self.save_font(ctx, name);
            }
            Message::FontInput(value) => {
                self.font_input = value;
                if self.installed.is_none() && !self.loading_fonts {
                    self.loading_fonts = true;
                    return Outcome::Task(Task::perform(
                        async {
                            tokio::task::spawn_blocking(font::installed_families)
                                .await
                                .unwrap_or_default()
                        },
                        Message::FontsLoaded,
                    ));
                }
            }
            Message::FontsLoaded(list) => {
                self.loading_fonts = false;
                self.installed = Some(list);
            }
            Message::FontApply => {
                if let Some(name) = self.font_matches().first().cloned() {
                    self.save_font(ctx, &name);
                }
            }
            Message::FontPick(name) => self.save_font(ctx, &name),
            Message::Restart => return Outcome::Restart,
        }
        Outcome::None
    }

    fn font_matches(&self) -> Vec<String> {
        let query = self.font_input.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let Some(installed) = &self.installed else {
            return Vec::new();
        };
        let mut matches: Vec<String> = installed
            .iter()
            .filter(|name| name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        matches.sort_by_key(|name| !name.to_lowercase().starts_with(&query));
        matches.truncate(40);
        matches
    }

    fn sync_picker(&mut self, ctx: &HubContext) {
        let palette = palette_for(&ctx.config().appearance);
        self.picker.set_color(self.slot.get(&palette));
    }

    fn set_color(&mut self, ctx: &HubContext, color: Color) {
        let slot = self.slot;
        self.save(ctx, |appearance| {
            let mut palette = palette_for(appearance);
            slot.set(&mut palette, color);
            appearance.theme = ThemeChoice::Custom;
            appearance.colors = colors_from(&palette);
        });
    }

    fn save_font(&mut self, ctx: &HubContext, name: &str) {
        let stored = if name == "Inter" {
            String::new()
        } else {
            name.to_string()
        };
        self.save(ctx, |appearance| appearance.font = stored.clone());
        self.font_saved = display_font(&stored);
    }

    fn save(&mut self, ctx: &HubContext, change: impl FnOnce(&mut AppearanceConfig)) {
        let mut config = ctx.config();
        change(&mut config.appearance);
        apply(&config.appearance);
        self.status = config
            .save(&ctx.paths.config_file())
            .err()
            .map(|error| format!("Não foi possível salvar: {error}"));
        *ctx.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
    }

    pub fn view<'a>(&'a self, ctx: &HubContext, zoom: f32) -> Element<'a, AppMessage> {
        let appearance = ctx.config().appearance;
        let current = palette_for(&appearance);
        let wrap = AppMessage::Appearance;

        let themes = row![
            choice(
                "Escuro",
                appearance.theme == ThemeChoice::Dark,
                wrap(Message::Theme(ThemeChoice::Dark))
            ),
            choice(
                "Claro",
                appearance.theme == ThemeChoice::Light,
                wrap(Message::Theme(ThemeChoice::Light))
            ),
            choice(
                "Personalizado",
                appearance.theme == ThemeChoice::Custom,
                wrap(Message::Theme(ThemeChoice::Custom))
            ),
        ]
        .spacing(units(2));

        let slots = ColorSlot::ALL
            .iter()
            .fold(Row::new().spacing(units(2)), |row, slot| {
                row.push(slot_chip(*slot, slot.get(&current), *slot == self.slot))
            });

        let selected = self.slot.get(&current);
        let mut colors = column![
            slots,
            color_picker::swatches(&self.slot.swatches(), selected, move |color| {
                wrap(Message::Swatch(color))
            }),
            row![
                ghost_button(
                    if self.advanced {
                        "Esconder personalização"
                    } else {
                        "Personalizar"
                    },
                    Some(wrap(Message::ToggleAdvanced))
                ),
                ghost_button("Restaurar cores", Some(wrap(Message::ResetCustom))),
            ]
            .spacing(units(2)),
        ]
        .spacing(units(3));
        if self.advanced {
            colors = colors.push(color_picker::view(&self.picker, move |message| {
                wrap(Message::Picker(message))
            }));
        }

        let percent = (zoom * 100.0).round() as i32;
        let zoom_row = row![
            ghost_button("Diminuir", Some(AppMessage::Zoom(-1))),
            body(format!("{percent}%"))
                .width(56)
                .align_x(Alignment::Center),
            ghost_button("Aumentar", Some(AppMessage::Zoom(1))),
            ghost_button("Restaurar", Some(AppMessage::ZoomReset)),
            muted("Ctrl +, Ctrl -, Ctrl 0"),
        ]
        .spacing(units(2))
        .align_y(Alignment::Center);

        let fonts = font::CHOICES
            .iter()
            .fold(Row::new().spacing(units(2)), |row, name| {
                row.push(font_chip(
                    name,
                    self.font_saved == *name,
                    wrap(Message::FontPreset(name)),
                ))
            });
        let preview = container(
            column![
                small("Prévia"),
                text("O rápido zorro marrom salta sobre o cão preguiçoso")
                    .size(20)
                    .font(font::family(&self.font_saved)),
                text("Agenda, atalhos e anotações em um só lugar. 0123456789")
                    .size(FONT_SIZE)
                    .font(font::family(&self.font_saved)),
            ]
            .spacing(units(2)),
        )
        .padding(units(4))
        .width(Length::Fill)
        .style(hub_ui_kit::style::elevated);
        let search = text_field("Procurar fonte instalada, por exemplo Cascadia", &self.font_input, move |value| {
            wrap(Message::FontInput(value))
        })
        .on_submit(wrap(Message::FontApply))
        .width(360);
        let matches = self.font_matches();
        let results: Element<'a, AppMessage> = if self.loading_fonts {
            muted("Carregando as fontes instaladas").into()
        } else if self.font_input.trim().is_empty() {
            muted(match &self.installed {
                Some(list) => format!("{} fontes instaladas. Digite parte do nome para procurar.", list.len()),
                None => "Digite parte do nome de uma fonte para procurar entre as instaladas no computador.".to_string(),
            })
            .into()
        } else if matches.is_empty() {
            muted("Nenhuma fonte instalada com esse nome.").into()
        } else {
            matches
                .into_iter()
                .fold(Row::new().spacing(units(2)), |row, name| {
                    let active = self.font_saved == name;
                    row.push(font_chip(font::intern(&name), active, wrap(Message::FontPick(name))))
                })
                .wrap()
                .into()
        };
        let mut font_section = column![
            heading("Fonte"),
            muted("Escolha uma das fontes mais usadas ou procure outra instalada no computador. Cada nome aparece escrito na própria fonte, e a prévia mostra a fonte escolhida. A troca vale depois de reiniciar o hub."),
            fonts.wrap(),
            preview,
            search,
            results,
        ]
        .spacing(units(3));
        if self.font_saved != self.font_running {
            font_section = font_section.push(
                row![
                    muted(format!("{} será usada ao reiniciar o hub", self.font_saved)),
                    primary_button("Reiniciar agora", Some(wrap(Message::Restart))),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center),
            );
        }

        let mut content = Column::new()
            .spacing(units(3))
            .push(heading("Tema"))
            .push(themes)
            .push(muted("Escolher uma cor a partir de Escuro ou Claro cria um tema personalizado com base nele."))
            .push(heading("Cores"))
            .push(colors)
            .push(space().height(units(2)))
            .push(self.wallpaper_section(&appearance))
            .push(space().height(units(2)))
            .push(heading("Zoom"))
            .push(zoom_row)
            .push(space().height(units(2)))
            .push(font_section);
        if let Some(status) = &self.status {
            content = content.push(body(status.as_str()).color(danger()));
        }
        content.into()
    }
}

impl Personalization {
    fn wallpaper_section<'a>(&'a self, appearance: &AppearanceConfig) -> Element<'a, AppMessage> {
        let wrap = AppMessage::Appearance;
        let config = &appearance.wallpaper;
        let mode = config.mode;
        let modes = [
            ("Nenhum", WallpaperMode::None),
            ("Imagem", WallpaperMode::Image),
            ("Papel de parede do Windows", WallpaperMode::System),
            ("Janela translúcida", WallpaperMode::Translucent),
        ]
        .into_iter()
        .fold(Row::new().spacing(units(2)), |row, (label, value)| {
            row.push(choice(
                label,
                mode == value,
                wrap(Message::WallpaperMode(value)),
            ))
        });
        let mut section = column![heading("Papel de parede"), modes.wrap()].spacing(units(3));
        if translucent_failed() {
            section = section.push(
                body("A janela translúcida não abriu neste computador e foi desligada para o hub continuar funcionando.")
                    .color(warning()),
            );
        }
        match mode {
            WallpaperMode::Image => {
                let name = std::path::Path::new(config.image.trim())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Nenhuma imagem escolhida".to_string());
                section = section.push(
                    row![
                        body(name),
                        ghost_button("Escolher imagem", Some(wrap(Message::PickImage))),
                    ]
                    .spacing(units(3))
                    .align_y(Alignment::Center),
                );
            }
            WallpaperMode::System => {
                section = section.push(muted(
                    "Usa a imagem atual da área de trabalho e acompanha quando ela muda.",
                ));
            }
            WallpaperMode::Translucent => {
                section = section.push(muted(
                    "A janela mostra o que estiver atrás dela, inclusive o Wallpaper Engine animado. Só funciona com o renderizador GPU.",
                ));
                let blurs = WindowBlur::ALL.iter().fold(
                    Row::new().spacing(units(2)).align_y(Alignment::Center).push(muted("Desfoque").width(90)),
                    |row, blur| {
                        row.push(choice(
                            blur.label(),
                            config.window_blur == *blur,
                            wrap(Message::WindowBlur(*blur)),
                        ))
                    },
                );
                section = section.push(blurs);
                section = section.push(muted(
                    "Leve e Forte usam o desfoque do próprio Windows, com intensidade fixa; o Forte é o acrílico.",
                ));
                if config.window_blur != WindowBlur::None && !platform::transparency_effects_enabled() {
                    section = section.push(
                        body("Os efeitos de transparência do Windows estão desligados, então o desfoque não aparece. Para ligar: Configurações do Windows, Personalização, Cores, Efeitos de transparência.")
                            .color(warning()),
                    );
                }
                if !translucent_session() {
                    section = section.push(
                        row![
                            muted("A janela translúcida vale ao reiniciar o hub"),
                            primary_button("Reiniciar agora", Some(wrap(Message::Restart))),
                        ]
                        .spacing(units(3))
                        .align_y(Alignment::Center),
                    );
                }
            }
            WallpaperMode::None => {}
        }
        if translucent_session() && mode != WallpaperMode::Translucent {
            section = section.push(
                row![
                    muted("A janela deixa de ser translúcida ao reiniciar o hub"),
                    ghost_button("Reiniciar agora", Some(wrap(Message::Restart))),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center),
            );
        }
        if mode != WallpaperMode::None {
            let dim = self.dim.unwrap_or(config.clamped_dim());
            section = section.push(
                row![
                    muted("Escurecer").width(90),
                    slider(0.0..=0.95, dim, move |value| wrap(Message::Dim(value)))
                        .step(0.01_f32)
                        .on_release(wrap(Message::DimReleased))
                        .width(260),
                    body(format!("{}%", (dim * 100.0).round() as i32)),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center),
            );
        }
        if matches!(mode, WallpaperMode::Image | WallpaperMode::System) {
            let blur = self.blur.unwrap_or(config.clamped_blur());
            section = section.push(
                row![
                    muted("Desfoque").width(90),
                    slider(0..=80, blur, move |value| wrap(Message::Blur(value)))
                        .on_release(wrap(Message::BlurReleased))
                        .width(260),
                    body(blur.to_string()),
                ]
                .spacing(units(3))
                .align_y(Alignment::Center),
            );
        }
        section.into()
    }
}

fn font_chip<'a>(name: &'static str, active: bool, message: AppMessage) -> Element<'a, AppMessage> {
    button(text(name).size(FONT_SIZE).font(font::family(name)))
        .padding([units(1) + 2.0, units(3)])
        .style(hub_ui_kit::style::toggle(active))
        .on_press(message)
        .into()
}

fn display_font(stored: &str) -> String {
    let trimmed = stored.trim();
    if trimmed.is_empty() {
        "Inter".to_string()
    } else {
        trimmed.to_string()
    }
}

fn slot_chip<'a>(slot: ColorSlot, color: Color, active: bool) -> Element<'a, AppMessage> {
    let dot = container(space())
        .width(12)
        .height(12)
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                color: border_subtle(),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });
    button(
        row![dot, text(slot.label()).size(FONT_SIZE)]
            .spacing(units(2))
            .align_y(Alignment::Center),
    )
    .padding([units(1) + 2.0, units(3)])
    .style(hub_ui_kit::style::toggle(active))
    .on_press(AppMessage::Appearance(Message::Slot(slot)))
    .width(Length::Shrink)
    .into()
}
