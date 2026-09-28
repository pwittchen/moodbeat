use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::llm::DEFAULT_MODEL;
use crate::paths::write_atomic;

pub const DEFAULT_CACHE_MAX_MB: u64 = 2048;

/// `~/.moodbeat/config.json` (SPEC §8.1).
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub openai: OpenAiConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub player: PlayerConfig,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CacheConfig {
    #[serde(default = "default_cache_max")]
    pub max_size_mb: u64,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_yt_dlp_update_check: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerConfig {
    /// Slider position, 0.0–1.0.
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self { volume: default_volume(), muted: false }
    }
}

fn default_volume() -> f64 {
    1.0
}

fn default_version() -> u32 {
    1
}
fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}
fn default_cache_max() -> u64 {
    DEFAULT_CACHE_MAX_MB
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            openai: OpenAiConfig::default(),
            cache: CacheConfig::default(),
            tools: ToolsConfig::default(),
            player: PlayerConfig::default(),
        }
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self { api_key: None, model: default_model() }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { max_size_mb: DEFAULT_CACHE_MAX_MB }
    }
}

// Hand-written so the key can never end up in logs via `{:?}`.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("version", &self.version)
            .field("has_api_key", &self.openai.api_key.is_some())
            .field("model", &self.openai.model)
            .field("cache_max_mb", &self.cache.max_size_mb)
            .field("volume", &self.player.volume)
            .field("muted", &self.player.muted)
            .finish()
    }
}

impl Config {
    /// Loads the config; a missing file yields defaults.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Atomic write with `0600` permissions.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).expect("config serializes");
        write_atomic(path, &json, Some(0o600))
    }

    /// Saved key, falling back to `OPENAI_API_KEY` from the environment.
    pub fn effective_api_key(&self) -> Option<String> {
        self.openai
            .api_key
            .clone()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.trim().is_empty()))
    }

    pub fn cache_max_bytes(&self) -> u64 {
        self.cache.max_size_mb * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(&dir.path().join("config.json")).unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.openai.model, DEFAULT_MODEL);
        assert_eq!(cfg.cache.max_size_mb, 2048);
        assert_eq!(cfg.player.volume, 1.0);
        assert!(!cfg.player.muted);
    }

    #[test]
    fn round_trip_with_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = Config::default();
        cfg.openai.api_key = Some("sk-test".into());
        cfg.openai.model = "gpt-x".into();
        cfg.player = PlayerConfig { volume: 0.35, muted: true };
        cfg.save(&path).unwrap();

        assert_eq!(Config::load(&path).unwrap(), cfg);
        assert!(!path.with_extension("tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn spec_example_parses() {
        let json = r#"{
          "version": 1,
          "openai": { "apiKey": "sk-...", "model": "gpt-4.1-mini" },
          "cache": { "maxSizeMb": 2048 },
          "tools": { "lastYtDlpUpdateCheck": "2026-09-28T19:00:00Z" }
        }"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.openai.api_key.as_deref(), Some("sk-..."));
        assert!(cfg.tools.last_yt_dlp_update_check.is_some());
    }

    #[test]
    fn debug_hides_key() {
        let mut cfg = Config::default();
        cfg.openai.api_key = Some("sk-secret".into());
        assert!(!format!("{cfg:?}").contains("sk-secret"));
    }
}
