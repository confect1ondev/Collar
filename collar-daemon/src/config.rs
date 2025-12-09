//! Configuration loading and management.

use anyhow::{Context, Result};
use collar_common::{Script, ScriptType};
use serde::Deserialize;
use std::path::Path;

/// Daemon configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Server connection settings.
    pub server: ServerConfig,

    /// Device identification.
    pub device: DeviceConfig,

    /// Status polling settings.
    #[serde(default)]
    pub polling: PollingConfig,

    /// Registered scripts.
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// WebSocket URL (e.g., "wss://your-server.com/ws").
    pub url: String,

    /// Device API key for authentication.
    pub device_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceConfig {
    /// Human-readable device name.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PollingConfig {
    /// Status poll interval in seconds.
    #[serde(default = "default_poll_interval")]
    pub interval_secs: u64,

    /// Scripts to run for status updates.
    #[serde(default)]
    pub status_scripts: Vec<String>,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_poll_interval(),
            status_scripts: Vec::new(),
        }
    }
}

fn default_poll_interval() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    pub script_type: ScriptTypeConfig,
    pub command: String,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptTypeConfig {
    Action,
    Status,
}

impl From<ScriptConfig> for Script {
    fn from(cfg: ScriptConfig) -> Self {
        Script {
            id: cfg.id,
            name: cfg.name,
            description: cfg.description,
            script_type: match cfg.script_type {
                ScriptTypeConfig::Action => ScriptType::Action,
                ScriptTypeConfig::Status => ScriptType::Status,
            },
            command: cfg.command,
            icon: cfg.icon,
        }
    }
}

impl Config {
    /// Load configuration from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config file: {:?}", path.as_ref()))?;

        toml::from_str(&content).context("Failed to parse config file")
    }

    /// Load from default locations.
    pub fn load_default() -> Result<Self> {
        let paths = [
            "collar.toml",
            "~/.config/collar/config.toml",
            "/etc/collar/config.toml",
        ];

        for path in paths {
            let expanded = shellexpand(path);
            if Path::new(&expanded).exists() {
                return Self::load(&expanded);
            }
        }

        anyhow::bail!("No configuration file found. Searched: {:?}", paths)
    }
}

fn shellexpand(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}{}", home.to_string_lossy(), &path[1..]);
        }
    }
    path.to_string()
}
