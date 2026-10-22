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
pub struct McpConfig {
    #[serde(rename = "porta")]
    pub port: u16,
    pub token: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            port: 47821,
            token: String::new(),
        }
    }
}

pub const ZOOM_MIN: f32 = 0.6;
pub const ZOOM_MAX: f32 = 2.0;
pub const ZOOM_STEP: f32 = 0.1;
pub const CHAT_WIDTH_MIN: f32 = 300.0;
pub const CHAT_WIDTH_MAX: f32 = 900.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub zoom: f32,
    #[serde(rename = "largura_chat")]
    pub chat_width: f32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            chat_width: 380.0,
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

    pub fn clamped_chat_width(&self) -> f32 {
        if self.chat_width.is_finite() {
            self.chat_width.clamp(CHAT_WIDTH_MIN, CHAT_WIDTH_MAX)
        } else {
            380.0
        }
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
    fn invalid_file_returns_parse_error() {
        let result = Config::parse(Path::new("x"), "[geral\nrenderizador = 3");
        assert!(matches!(result, Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn saved_file_uses_portuguese_keys() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(text.contains("[geral]"));
        assert!(text.contains("renderizador = \"gpu\""));
        assert!(text.contains("porta = 47821"));
    }
}
