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
    #[serde(default)]
    pub homekit: Option<HomeKitConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Path to the persisted state file. If unset, state is not persisted across restarts.
    #[serde(default)]
    pub state_path: Option<String>,
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
    /// Optional MAC address used for Wake-on-LAN. When set, the Homebridge
    /// plugin can wake the device by sending a magic packet on the local
    /// LAN. Format: 12 hex digits, optionally separated by `:` or `-`.
    #[serde(default)]
    pub wol_mac: Option<String>,
}

/// HomeKit / Homebridge integration config. Optional — when absent, the
/// `/api/homekit` surface is unavailable.
#[derive(Debug, Clone, Deserialize)]
pub struct HomeKitConfig {
    /// Dedicated API key for the Homebridge plugin (not a user JWT).
    pub api_key: String,
    /// Switch definitions: each maps an on/off/state script triple on a daemon
    /// to a single HomeKit Switch accessory.
    #[serde(default)]
    pub switches: Vec<HomeKitSwitchConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HomeKitSwitchConfig {
    /// User-facing identifier. Cosmetic — renaming it doesn't re-pair the
    /// accessory (HomeKit identity is derived from device_id + behaviour).
    pub id: String,
    /// Display name shown in the Home app for this individual service.
    pub name: String,
    /// Target device UUID.
    pub device_id: String,
    /// HomeKit service type. `switch` (default) exposes a plain on/off
    /// toggle. `lock` exposes a HomeKit LockMechanism — proper lock icon,
    /// Siri "lock my …" support, Locked/Unlocked terminology.
    #[serde(default)]
    pub accessory_type: collar_common::HomeKitAccessoryType,
    /// Script id (declared by the daemon) to execute when HomeKit turns the switch ON.
    /// For `lock`-type entries this is the *lock* action.
    pub on_script: String,
    /// Script id to execute when HomeKit turns the switch OFF.
    /// For `lock`-type entries this is the *unlock* action.
    pub off_script: String,
    /// Where the switch's on/off state is read from. Defaults to `Status`
    /// (poll a status script on the daemon). Use `Online` for switches whose
    /// state is "PC is alive" — e.g. a power switch that flips to OFF the
    /// moment the daemon disconnects after a shutdown command.
    #[serde(default)]
    pub state_source: SwitchStateSource,
    /// Script id whose output represents the current on/off state. Required
    /// when `state_source = "status"` (the default). Must also appear in the
    /// daemon's `polling.status_scripts` for live updates.
    #[serde(default)]
    pub state_script: Option<String>,
    /// The string value (case-insensitive) that `state_script` emits when the
    /// switch should read as ON. Required when `state_source = "status"`.
    #[serde(default)]
    pub state_on_value: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwitchStateSource {
    /// Read state from a script the daemon polls. Requires `state_script`
    /// and `state_on_value`. This is the default and the right choice for
    /// most switches (lock, mute, etc.).
    #[default]
    Status,
    /// State mirrors whether the daemon is currently connected. The switch
    /// reads ON while the WebSocket is up and flips to OFF the instant the
    /// daemon disconnects. `state_script` and `state_on_value` are ignored.
    Online,
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
        let cfg: Config = toml::from_str(&content).context("Failed to parse config")?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject obvious misconfigurations at startup rather than surface them
    /// as confusing 404s later.
    fn validate(&self) -> Result<()> {
        for dk in &self.devices {
            uuid::Uuid::parse_str(&dk.device_id).with_context(|| {
                format!(
                    "device `{}` has device_id `{}` which is not a valid UUID",
                    dk.name, dk.device_id
                )
            })?;
            if let Some(mac) = &dk.wol_mac {
                parse_mac(mac).with_context(|| {
                    format!("device `{}` has invalid wol_mac `{mac}`", dk.name)
                })?;
            }
        }

        if let Some(hk) = &self.homekit {
            if hk.api_key.trim().is_empty() {
                anyhow::bail!("[homekit].api_key must not be empty");
            }

            let known_devices: std::collections::HashSet<&str> =
                self.devices.iter().map(|d| d.device_id.as_str()).collect();
            let mut seen_ids = std::collections::HashSet::new();

            for sw in &hk.switches {
                if !seen_ids.insert(sw.id.as_str()) {
                    anyhow::bail!("duplicate [[homekit.switches]] id `{}`", sw.id);
                }
                if !known_devices.contains(sw.device_id.as_str()) {
                    anyhow::bail!(
                        "switch `{}` references device_id `{}` which is not in [[devices]]",
                        sw.id,
                        sw.device_id
                    );
                }
                if matches!(sw.state_source, SwitchStateSource::Status)
                    && (sw.state_script.is_none() || sw.state_on_value.is_none())
                {
                    anyhow::bail!(
                        "switch `{}` uses state_source=\"status\" but is missing state_script or state_on_value",
                        sw.id
                    );
                }
            }
        }

        Ok(())
    }
}

/// 12 hex digits with optional `:`, `-`, or `.` separators.
fn parse_mac(s: &str) -> Result<[u8; 6]> {
    let hex: String = s.chars().filter(|c| !matches!(c, ':' | '-' | '.')).collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("expected 12 hex digits (with optional separators)");
    }
    let mut out = [0u8; 6];
    for i in 0..6 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use collar_common::HomeKitAccessoryType;

    fn base() -> Config {
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                state_path: None,
            },
            auth: AuthConfig {
                jwt_secret: "x".to_string(),
                jwt_expiry_hours: 1,
                admin_username: "a".to_string(),
                admin_password_hash: "h".to_string(),
            },
            devices: vec![DeviceKeyConfig {
                device_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
                name: "pc".to_string(),
                api_key: "k".to_string(),
                wol_mac: None,
            }],
            homekit: None,
        }
    }

    fn switch(id: &str, dev: &str) -> HomeKitSwitchConfig {
        HomeKitSwitchConfig {
            id: id.to_string(),
            name: id.to_string(),
            device_id: dev.to_string(),
            accessory_type: HomeKitAccessoryType::Switch,
            on_script: "on".to_string(),
            off_script: "off".to_string(),
            state_source: SwitchStateSource::Status,
            state_script: Some("s".to_string()),
            state_on_value: Some("yes".to_string()),
        }
    }

    #[test]
    fn invalid_device_uuid_rejected() {
        let mut c = base();
        c.devices[0].device_id = "not-a-uuid".to_string();
        assert!(c.validate().is_err());
    }

    #[test]
    fn switch_with_unknown_device_rejected() {
        let mut c = base();
        c.homekit = Some(HomeKitConfig {
            api_key: "hk".to_string(),
            switches: vec![switch("a", "11111111-1111-1111-1111-111111111111")],
        });
        let err = c.validate().unwrap_err().to_string();
        assert!(err.contains("not in [[devices]]"), "got: {err}");
    }

    #[test]
    fn duplicate_switch_id_rejected() {
        let mut c = base();
        let d = c.devices[0].device_id.clone();
        c.homekit = Some(HomeKitConfig {
            api_key: "hk".to_string(),
            switches: vec![switch("dup", &d), switch("dup", &d)],
        });
        assert!(c.validate().unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn status_switch_requires_state_script() {
        let mut c = base();
        let d = c.devices[0].device_id.clone();
        let mut sw = switch("a", &d);
        sw.state_script = None;
        c.homekit = Some(HomeKitConfig {
            api_key: "hk".to_string(),
            switches: vec![sw],
        });
        assert!(c.validate().is_err());
    }

    #[test]
    fn online_switch_does_not_require_state_script() {
        let mut c = base();
        let d = c.devices[0].device_id.clone();
        let mut sw = switch("p", &d);
        sw.state_source = SwitchStateSource::Online;
        sw.state_script = None;
        sw.state_on_value = None;
        c.homekit = Some(HomeKitConfig {
            api_key: "hk".to_string(),
            switches: vec![sw],
        });
        assert!(c.validate().is_ok());
    }

    #[test]
    fn empty_homekit_api_key_rejected() {
        let mut c = base();
        c.homekit = Some(HomeKitConfig {
            api_key: "   ".to_string(),
            switches: vec![],
        });
        assert!(c.validate().is_err());
    }

    #[test]
    fn bad_wol_mac_rejected() {
        let mut c = base();
        c.devices[0].wol_mac = Some("zz:zz:zz:zz:zz:zz".to_string());
        assert!(c.validate().is_err());
    }

    #[test]
    fn good_wol_mac_accepted() {
        for mac in ["aa:bb:cc:dd:ee:ff", "AA-BB-CC-DD-EE-FF", "aabbccddeeff"] {
            let mut c = base();
            c.devices[0].wol_mac = Some(mac.to_string());
            assert!(c.validate().is_ok(), "rejected valid mac {mac}");
        }
    }
}
