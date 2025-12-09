//! Server configuration.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub devices: Vec<DeviceKeyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// JWT secret key.
    pub jwt_secret: String,
    /// JWT expiry in hours.
    #[serde(default = "default_jwt_expiry")]
    pub jwt_expiry_hours: u64,
    /// Admin username.
    pub admin_username: String,
    /// Admin password hash (argon2).
    pub admin_password_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceKeyConfig {
    pub device_id: String,
    pub name: String,
    pub api_key: String,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    4221
}

fn default_jwt_expiry() -> u64 {
    24
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read config: {:?}", path.as_ref()))?;
        toml::from_str(&content).context("Failed to parse config")
    }
}
