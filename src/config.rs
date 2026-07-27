use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub whisper: WhisperConfig,
    pub overlay: OverlayConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Key name as understood by evdev, with or without the KEY_ prefix (e.g. "F12").
    pub key: String,
    pub mode: HotkeyMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HotkeyMode {
    /// Hold to record, release to transcribe.
    PushToTalk,
    /// Press once to start, press again to stop.
    Toggle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// "default" or an exact input device name (see `quickwhisper status`).
    pub device: String,
    /// Safety cap for stuck keys / forgotten recordings.
    pub max_recording_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperConfig {
    /// ggml model name: tiny, base, small, medium, large-v3…
    pub model: String,
    /// "auto" for detection, or a fixed code like "pt"/"en".
    pub language: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayMonitor {
    #[default]
    Pointer,
    Primary,
    All,
}

impl OverlayMonitor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Primary => "primary",
            Self::All => "all",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "pointer" => Ok(Self::Pointer),
            "primary" => Ok(Self::Primary),
            "all" => Ok(Self::All),
            _ => anyhow::bail!(
                "monitor inválido '{value}' (use pointer, primary ou all)"
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OverlayConfig {
    pub monitor: OverlayMonitor,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self { key: "F12".into(), mode: HotkeyMode::PushToTalk }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { device: "default".into(), max_recording_secs: 120 }
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self { model: "small".into(), language: "pt".into() }
    }
}

impl Config {
    /// Loads the config file, creating one with defaults on first run so the
    /// user has something discoverable to edit.
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            let cfg = Self::default();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let body = toml::to_string_pretty(&cfg)?;
            std::fs::write(&path, body)
                .with_context(|| format!("gravando config padrão em {}", path.display()))?;
            return Ok(cfg);
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("lendo {}", path.display()))?;
        toml::from_str(&body).with_context(|| format!("parseando {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("gravando {}", path.display()))
    }

    /// Resolves a CLI key (dotted path or shorthand) and returns its value.
    pub fn get_by_key(&self, key: &str) -> Result<String> {
        Ok(match canonical_key(key)? {
            "hotkey.key" => self.hotkey.key.clone(),
            "hotkey.mode" => match self.hotkey.mode {
                HotkeyMode::PushToTalk => "push-to-talk".into(),
                HotkeyMode::Toggle => "toggle".into(),
            },
            "audio.device" => self.audio.device.clone(),
            "audio.max_recording_secs" => self.audio.max_recording_secs.to_string(),
            "whisper.model" => self.whisper.model.clone(),
            "whisper.language" => self.whisper.language.clone(),
            "overlay.monitor" => self.overlay.monitor.as_str().into(),
            _ => unreachable!("canonical_key só retorna chaves conhecidas"),
        })
    }

    /// Sets a config value from CLI strings, with validation per field.
    pub fn set_by_key(&mut self, key: &str, value: &str) -> Result<()> {
        match canonical_key(key)? {
            "hotkey.key" => {
                crate::hotkey::parse_key(value)?; // validates
                self.hotkey.key = value.to_owned();
            }
            "hotkey.mode" => {
                self.hotkey.mode = match value {
                    "push-to-talk" => HotkeyMode::PushToTalk,
                    "toggle" => HotkeyMode::Toggle,
                    _ => anyhow::bail!("modo inválido '{value}' (use push-to-talk ou toggle)"),
                };
            }
            "audio.device" => self.audio.device = value.to_owned(),
            "audio.max_recording_secs" => {
                self.audio.max_recording_secs =
                    value.parse().context("max_recording_secs deve ser um número de segundos")?;
            }
            "whisper.model" => self.whisper.model = value.to_owned(),
            "whisper.language" => {
                // "auto" enables whisper's detection; anything else must be a
                // language code whisper knows (pt, en, es, …).
                anyhow::ensure!(
                    value == "auto" || whisper_rs::get_lang_id(value).is_some(),
                    "idioma inválido '{value}' (use um código como pt/en/es, ou auto)"
                );
                self.whisper.language = value.to_owned();
            }
            "overlay.monitor" => {
                self.overlay.monitor = OverlayMonitor::parse(value)?;
            }
            _ => unreachable!("canonical_key só retorna chaves conhecidas"),
        }
        Ok(())
    }
}

/// Accepts both full dotted paths and unambiguous shorthands.
fn canonical_key(key: &str) -> Result<&'static str> {
    Ok(match key {
        "hotkey.key" | "key" => "hotkey.key",
        "hotkey.mode" | "mode" => "hotkey.mode",
        "audio.device" | "device" => "audio.device",
        "audio.max_recording_secs" | "max_recording_secs" => "audio.max_recording_secs",
        "whisper.model" | "model" => "whisper.model",
        "whisper.language" | "language" | "lang" => "whisper.language",
        "overlay.monitor" | "monitor" => "overlay.monitor",
        _ => anyhow::bail!(
            "chave desconhecida '{key}'. Válidas: key, mode, device, max_recording_secs, model, language, monitor"
        ),
    })
}

pub fn config_path() -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join("config.toml"))
}

pub fn models_dir() -> Result<PathBuf> {
    Ok(project_dirs()?.data_dir().join("models"))
}

pub fn history_db_path() -> Result<PathBuf> {
    Ok(project_dirs()?.data_dir().join("history.db"))
}

fn project_dirs() -> Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", "quickwhisper")
        .context("não foi possível resolver os diretórios do usuário (HOME ausente?)")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_CONFIG: &str = r#"
[hotkey]
key = "F12"
mode = "push-to-talk"

[audio]
device = "default"
max_recording_secs = 120

[whisper]
model = "small"
language = "pt"
"#;

    #[test]
    fn legacy_config_defaults_overlay_monitor_to_pointer() {
        let cfg: Config = toml::from_str(LEGACY_CONFIG).expect("config antiga deve ser válida");

        assert_eq!(cfg.overlay.monitor, OverlayMonitor::Pointer);
    }

    #[test]
    fn overlay_monitor_accepts_supported_values() {
        for (raw, expected) in [
            ("pointer", OverlayMonitor::Pointer),
            ("primary", OverlayMonitor::Primary),
            ("all", OverlayMonitor::All),
        ] {
            let body = format!("[overlay]\nmonitor = \"{raw}\"\n");
            let cfg: Config = toml::from_str(&body).expect("modo suportado deve ser válido");

            assert_eq!(cfg.overlay.monitor, expected);
        }
    }

    #[test]
    fn overlay_monitor_rejects_unknown_toml_value() {
        let result = toml::from_str::<Config>("[overlay]\nmonitor = \"primay\"\n");

        assert!(result.is_err());
    }

    #[test]
    fn config_get_and_set_overlay_monitor() {
        let mut cfg = Config::default();

        assert_eq!(cfg.get_by_key("overlay.monitor").unwrap(), "pointer");

        cfg.set_by_key("monitor", "all").unwrap();

        assert_eq!(cfg.overlay.monitor, OverlayMonitor::All);
        assert_eq!(cfg.get_by_key("monitor").unwrap(), "all");

        let error = cfg.set_by_key("overlay.monitor", "primay").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("monitor inválido 'primay' (use pointer, primary ou all)")
        );
    }

    #[test]
    fn default_config_serializes_overlay_section() {
        let body = toml::to_string_pretty(&Config::default())
            .expect("config padrão deve ser serializável");

        assert!(body.contains("[overlay]"));
        assert!(body.contains("monitor = \"pointer\""));
    }
}
