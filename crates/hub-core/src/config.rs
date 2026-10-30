use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("falha ao ler {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("falha ao gravar {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("arquivo {path} inválido: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("falha ao serializar a configuração: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    #[default]
    Gpu,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    #[serde(rename = "renderizador")]
    pub renderer: Renderer,
    #[serde(rename = "iniciar_com_sistema")]
    pub start_with_system: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            renderer: Renderer::Gpu,
            start_with_system: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LedsConfig {
    #[serde(rename = "porta")]
    pub port: u16,
    #[serde(rename = "perfil_modo_jogo")]
    pub game_profile: String,
    #[serde(rename = "perfil_normal")]
    pub normal_profile: String,
}

impl Default for LedsConfig {
    fn default() -> Self {
        Self {
            port: 6742,
            game_profile: String::new(),
            normal_profile: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    #[serde(rename = "porta")]
    pub port: u16,
    pub token: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            port: crate::channel::Channel::current().default_mcp_port(),
            token: String::new(),
        }
    }
}

pub const ZOOM_MIN: f32 = 0.6;
pub const ZOOM_MAX: f32 = 2.0;
pub const ZOOM_STEP: f32 = 0.1;
pub const CHAT_WIDTH_MIN: f32 = 300.0;
pub const CHAT_WIDTH_MAX: f32 = 900.0;
pub const TERMINAL_HEIGHT_MIN: f32 = 120.0;
pub const TERMINAL_HEIGHT_DEFAULT: f32 = 260.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub zoom: f32,
    #[serde(rename = "largura_chat")]
    pub chat_width: f32,
    #[serde(rename = "altura_terminal")]
    pub terminal_height: f32,
    #[serde(rename = "tema")]
    pub theme: ThemeChoice,
    #[serde(rename = "cores")]
    pub colors: CustomColors,
    #[serde(rename = "fonte")]
    pub font: String,
    #[serde(rename = "papel_de_parede")]
    pub wallpaper: WallpaperConfig,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            chat_width: 380.0,
            terminal_height: TERMINAL_HEIGHT_DEFAULT,
            theme: ThemeChoice::Dark,
            colors: CustomColors::default(),
            font: String::new(),
            wallpaper: WallpaperConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WallpaperMode {
    #[default]
    #[serde(rename = "nenhum")]
    None,
    #[serde(rename = "imagem")]
    Image,
    #[serde(rename = "sistema")]
    System,
    #[serde(rename = "translucido")]
    Translucent,
}

pub const WALLPAPER_BLUR_MAX: u32 = 80;
pub const WALLPAPER_DIM_MAX: f32 = 0.95;
const WALLPAPER_DIM_DEFAULT: f32 = 0.55;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    #[serde(rename = "modo")]
    pub mode: WallpaperMode,
    #[serde(rename = "imagem")]
    pub image: String,
    #[serde(rename = "desfoque")]
    pub blur: u32,
    #[serde(rename = "escurecer")]
    pub dim: f32,
    #[serde(rename = "desfoque_janela")]
    pub window_blur: WindowBlur,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowBlur {
    #[default]
    #[serde(rename = "nenhum")]
    None,
    #[serde(rename = "leve")]
    Light,
    #[serde(rename = "forte")]
    Strong,
}

impl WindowBlur {
    pub const ALL: [WindowBlur; 3] = [WindowBlur::None, WindowBlur::Light, WindowBlur::Strong];

    pub fn label(self) -> &'static str {
        match self {
            WindowBlur::None => "Nenhum",
            WindowBlur::Light => "Leve",
            WindowBlur::Strong => "Forte",
        }
    }
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            mode: WallpaperMode::None,
            image: String::new(),
            blur: 30,
            dim: WALLPAPER_DIM_DEFAULT,
            window_blur: WindowBlur::None,
        }
    }
}

impl WallpaperConfig {
    pub fn clamped_blur(&self) -> u32 {
        self.blur.min(WALLPAPER_BLUR_MAX)
    }

    pub fn clamped_dim(&self) -> f32 {
        if self.dim.is_finite() {
            self.dim.clamp(0.0, WALLPAPER_DIM_MAX)
        } else {
            WALLPAPER_DIM_DEFAULT
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    #[serde(rename = "escuro")]
    Dark,
    #[serde(rename = "claro")]
    Light,
    #[serde(rename = "personalizado")]
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomColors {
    #[serde(rename = "fundo")]
    pub background: String,
    #[serde(rename = "painel")]
    pub surface: String,
    #[serde(rename = "texto")]
    pub text: String,
    #[serde(rename = "destaque")]
    pub accent: String,
    #[serde(rename = "secundaria")]
    pub secondary: String,
}

impl Default for CustomColors {
    fn default() -> Self {
        Self {
            background: "#111214".to_string(),
            surface: "#1A1B1E".to_string(),
            text: "#E6E6E7".to_string(),
            accent: "#7C9CF5".to_string(),
            secondary: "#B48CF2".to_string(),
        }
    }
}

impl AppearanceConfig {
    pub fn clamped_zoom(&self) -> f32 {
        if self.zoom.is_finite() {
            self.zoom.clamp(ZOOM_MIN, ZOOM_MAX)
        } else {
            1.0
        }
    }

    pub fn step_zoom(&mut self, steps: i32) {
        let next = self.clamped_zoom() + ZOOM_STEP * steps as f32;
        self.zoom = ((next * 10.0).round() / 10.0).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    pub fn clamped_terminal_height(&self) -> f32 {
        if self.terminal_height.is_finite() {
            self.terminal_height.max(TERMINAL_HEIGHT_MIN)
        } else {
            TERMINAL_HEIGHT_DEFAULT
        }
    }

    pub fn clamped_chat_width(&self) -> f32 {
        if self.chat_width.is_finite() {
            self.chat_width.clamp(CHAT_WIDTH_MIN, CHAT_WIDTH_MAX)
        } else {
            380.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    #[serde(rename = "automatico")]
    pub automatic: bool,
    #[serde(rename = "manter")]
    pub keep: u32,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            automatic: true,
            keep: 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    #[serde(rename = "pasta")]
    pub folder: String,
    #[serde(rename = "verificar_ao_abrir")]
    pub check_on_start: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            folder: String::new(),
            check_on_start: true,
        }
    }
}

impl UpdateConfig {
    pub fn update_folder(&self) -> PathBuf {
        let configured = self.folder.trim();
        if !configured.is_empty() {
            return PathBuf::from(configured);
        }
        let base = directories::UserDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_default();
        base.join("HubPessoal").join("atualizacoes")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(rename = "geral")]
    pub general: GeneralConfig,
    pub mcp: McpConfig,
    #[serde(rename = "aparencia")]
    pub appearance: AppearanceConfig,
    pub backup: BackupConfig,
    #[serde(rename = "atualizacao")]
    pub update: UpdateConfig,
    pub player: PlayerConfig,
    #[serde(rename = "ia")]
    pub ai: AiConfig,
    pub leds: LedsConfig,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiProviderKind {
    #[default]
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "api")]
    Api,
}

impl AiProviderKind {
    pub const ALL: [AiProviderKind; 4] = [
        AiProviderKind::Claude,
        AiProviderKind::Codex,
        AiProviderKind::Gemini,
        AiProviderKind::Api,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AiProviderKind::Claude => "Claude CLI",
            AiProviderKind::Codex => "Codex CLI",
            AiProviderKind::Gemini => "Gemini CLI",
            AiProviderKind::Api => "API ou modelo local",
        }
    }
}

pub const AI_API_URL_DEFAULT: &str = "http://localhost:11434/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    #[serde(rename = "provedor")]
    pub provider: AiProviderKind,
    pub api_url: String,
    #[serde(rename = "api_modelo")]
    pub api_model: String,
    #[serde(rename = "api_chave")]
    pub api_key: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::Claude,
            api_url: AI_API_URL_DEFAULT.to_string(),
            api_model: String::new(),
            api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerConfig {
    #[serde(rename = "login_navegador")]
    pub browser_login: BrowserLogin,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserLogin {
    #[default]
    #[serde(rename = "nenhum")]
    None,
    #[serde(rename = "zen")]
    Zen,
    #[serde(rename = "firefox")]
    Firefox,
    #[serde(rename = "chrome")]
    Chrome,
    #[serde(rename = "edge")]
    Edge,
    #[serde(rename = "brave")]
    Brave,
}

impl BrowserLogin {
    pub const ALL: [BrowserLogin; 6] = [
        BrowserLogin::None,
        BrowserLogin::Zen,
        BrowserLogin::Firefox,
        BrowserLogin::Chrome,
        BrowserLogin::Edge,
        BrowserLogin::Brave,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BrowserLogin::None => "Nenhum",
            BrowserLogin::Zen => "Zen",
            BrowserLogin::Firefox => "Firefox",
            BrowserLogin::Chrome => "Chrome",
            BrowserLogin::Edge => "Edge",
            BrowserLogin::Brave => "Brave",
        }
    }
}

impl Config {
    pub fn parse(path: &Path, text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(path, &text)
    }

    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if path.exists() {
            return Self::load(path);
        }
        let config = Self::default();
        config.save(path)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_created_with_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let config = Config::load_or_create(&path).unwrap();
        assert_eq!(config, Config::default());
        assert!(path.exists());
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn partial_file_uses_defaults_for_missing_fields() {
        let config =
            Config::parse(Path::new("x"), "[geral]\nrenderizador = \"software\"\n").unwrap();
        assert_eq!(config.general.renderer, Renderer::Software);
        assert!(!config.general.start_with_system);
        assert_eq!(config.mcp, McpConfig::default());
    }

    #[test]
    fn leds_defaults_and_overrides() {
        let config = Config::default();
        assert_eq!(config.leds.port, 6742);
        assert!(config.leds.game_profile.is_empty());
        let parsed = Config::parse(
            Path::new("x"),
            "[leds]
porta = 7000
perfil_modo_jogo = \"Jogo\"
",
        )
        .unwrap();
        assert_eq!(parsed.leds.port, 7000);
        assert_eq!(parsed.leds.game_profile, "Jogo");
        assert!(parsed.leds.normal_profile.is_empty());
    }

    #[test]
    fn missing_renderer_defaults_to_gpu() {
        let config = Config::parse(Path::new("x"), "").unwrap();
        assert_eq!(config.general.renderer, Renderer::Gpu);
    }

    #[test]
    fn zoom_steps_are_rounded_and_limited() {
        let mut appearance = AppearanceConfig::default();
        appearance.step_zoom(1);
        assert_eq!(appearance.zoom, 1.1);
        appearance.step_zoom(-5);
        assert_eq!(appearance.zoom, 0.6);
        appearance.step_zoom(-1);
        assert_eq!(appearance.zoom, ZOOM_MIN);
        appearance.zoom = 50.0;
        assert_eq!(appearance.clamped_zoom(), ZOOM_MAX);
        appearance.chat_width = 10.0;
        assert_eq!(appearance.clamped_chat_width(), CHAT_WIDTH_MIN);
        let parsed = Config::parse(
            Path::new("x"),
            "[aparencia]
zoom = 1.3
",
        )
        .unwrap();
        assert_eq!(parsed.appearance.zoom, 1.3);
        assert_eq!(parsed.appearance.chat_width, 380.0);
    }

    #[test]
    fn wallpaper_defaults_and_limits() {
        let parsed = Config::parse(Path::new("x"), "").unwrap();
        assert_eq!(parsed.appearance.wallpaper, WallpaperConfig::default());
        assert_eq!(parsed.appearance.wallpaper.mode, WallpaperMode::None);
        let parsed = Config::parse(
            Path::new("x"),
            "[aparencia.papel_de_parede]\nmodo = \"translucido\"\ndesfoque = 500\nescurecer = 3.0\n",
        )
        .unwrap();
        let wallpaper = parsed.appearance.wallpaper;
        assert_eq!(wallpaper.mode, WallpaperMode::Translucent);
        assert_eq!(wallpaper.clamped_blur(), 80);
        assert_eq!(wallpaper.clamped_dim(), 0.95);
        let broken = WallpaperConfig {
            dim: f32::NAN,
            ..WallpaperConfig::default()
        };
        assert_eq!(broken.clamped_dim(), 0.55);
    }

    #[test]
    fn invalid_file_returns_parse_error() {
        let result = Config::parse(Path::new("x"), "[geral\nrenderizador = 3");
        assert!(matches!(result, Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn saved_file_uses_portuguese_keys() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("[geral]"));
        assert!(text.contains("renderizador = \"gpu\""));
        let port = crate::channel::Channel::current().default_mcp_port();
        assert!(text.contains(&format!("porta = {port}")));
    }
}
