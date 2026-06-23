//! Application state and device management.

use chrono::{DateTime, Utc};
use collar_common::{
    homekit_device_accessory_uuid, homekit_service_subtype, Device, DeviceId, DeviceStatus,
    HomeKitAccessoryType, HomeKitEvent, HomeKitSwitchState, ScriptInfo, ServerMessage,
};
use dashmap::DashMap;
use serde_json::Value;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::info;
use uuid::Uuid;

use crate::config::{Config, HomeKitSwitchConfig, SwitchStateSource};
use crate::persistence::{PersistedDevice, PersistedState, StatePersister};

/// Capacity of the HomeKit event broadcast channel. With per-switch events
/// emitted on every status update, this comfortably absorbs bursts. Slow
/// consumers that fall behind will see `RecvError::Lagged` and resync via a
/// fresh `GET /switches` poll on the plugin side.
const HOMEKIT_EVENT_BUFFER: usize = 256;

/// Connected device with its sender channel.
pub struct ConnectedDevice {
    pub id: DeviceId,
    pub name: String,
    pub connected_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub status: DeviceStatus,
    pub status_observed_at: Option<DateTime<Utc>>,
    pub scripts: Vec<ScriptInfo>,
    /// LAN IP the daemon reported at connect. Sticky — preserved across
    /// disconnect/reconnect so WoL targets the last-known IP even while the
    /// device is offline.
    pub lan_ip: Option<IpAddr>,
    pub tx: mpsc::Sender<ServerMessage>,
    /// Per-connection token. The handler stashes this and presents it on
    /// unregister so a late TCP-close from a dead connection can't kick a
    /// legitimate reconnect that took its place.
    pub session_id: Uuid,
}

/// Cached info for recently disconnected devices.
#[derive(Clone)]
pub struct OfflineDevice {
    pub id: DeviceId,
    pub name: String,
    pub disconnected_at: DateTime<Utc>,
    pub last_status: DeviceStatus,
    pub status_observed_at: Option<DateTime<Utc>>,
    pub scripts: Vec<ScriptInfo>,
    pub lan_ip: Option<IpAddr>,
}

/// Cheap snapshot of a device's identity + observed status, suitable for
/// computing derived views (e.g. switch states) without holding any DashMap
/// guards across awaits.
struct DeviceSnapshot {
    name: String,
    online: bool,
    status: DeviceStatus,
    status_observed_at: Option<DateTime<Utc>>,
    lan_ip: Option<IpAddr>,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub devices: Arc<DashMap<DeviceId, ConnectedDevice>>,
    pub offline_devices: Arc<DashMap<DeviceId, OfflineDevice>>,
    pub device_keys: Arc<DashMap<String, (DeviceId, String)>>, // api_key -> (device_id, name)
    pub persister: Option<Arc<StatePersister>>,
    pub homekit_events: broadcast::Sender<HomeKitEvent>,
}

impl AppState {
    pub fn new(config: Config, persister: Option<Arc<StatePersister>>) -> Self {
        let device_keys = DashMap::new();
        for dk in &config.devices {
            // Config validation rejects bad UUIDs before we get here, but be
            // defensive: skip rather than silently remap to a random UUID
            // (which used to detach api_keys from real devices).
            match Uuid::parse_str(&dk.device_id) {
                Ok(id) => {
                    device_keys.insert(dk.api_key.clone(), (id, dk.name.clone()));
                }
                Err(_) => {
                    tracing::error!(
                        device = %dk.name,
                        device_id = %dk.device_id,
                        "Skipping device with invalid UUID",
                    );
                }
            }
        }

        let (homekit_events, _) = broadcast::channel(HOMEKIT_EVENT_BUFFER);

        Self {
            config: Arc::new(config),
            devices: Arc::new(DashMap::new()),
            offline_devices: Arc::new(DashMap::new()),
            device_keys: Arc::new(device_keys),
            persister,
            homekit_events,
        }
    }

    /// Subscribe to HomeKit events. Drop the receiver to unsubscribe.
    pub fn subscribe_homekit(&self) -> broadcast::Receiver<HomeKitEvent> {
        self.homekit_events.subscribe()
    }

    /// Restore offline_devices from a previously persisted snapshot.
    /// Called once at startup before any daemons connect.
    pub fn restore(&self, state: PersistedState) {
        for d in state.devices {
            let lan_ip = d.lan_ip.as_deref().and_then(|s| s.parse::<IpAddr>().ok());
            self.offline_devices.insert(
                d.id,
                OfflineDevice {
                    id: d.id,
                    name: d.name,
                    disconnected_at: d.last_seen,
                    last_status: d.last_status,
                    status_observed_at: d.status_observed_at,
                    scripts: d.scripts,
                    lan_ip,
                },
            );
        }
        info!(
            count = self.offline_devices.len(),
            "Restored devices from persisted state"
        );
    }

    /// Snapshot the union of online + offline devices for persistence.
    pub fn snapshot(&self) -> PersistedState {
        let mut devices = Vec::new();

        for r in self.devices.iter() {
            let d = r.value();
            devices.push(PersistedDevice {
                id: d.id,
                name: d.name.clone(),
                last_seen: d.last_seen,
                last_status: d.status.clone(),
                status_observed_at: d.status_observed_at,
                scripts: d.scripts.clone(),
                lan_ip: d.lan_ip.map(|ip| ip.to_string()),
            });
        }

        for r in self.offline_devices.iter() {
            let d = r.value();
            devices.push(PersistedDevice {
                id: d.id,
                name: d.name.clone(),
                last_seen: d.disconnected_at,
                last_status: d.last_status.clone(),
                status_observed_at: d.status_observed_at,
                scripts: d.scripts.clone(),
                lan_ip: d.lan_ip.map(|ip| ip.to_string()),
            });
        }

        PersistedState {
            version: 1,
            devices,
        }
    }

    async fn persist(&self) {
        if let Some(p) = &self.persister {
            let snapshot = self.snapshot();
            p.save_best_effort(&snapshot).await;
        }
    }

    /// Validate a device API key and return device info.
    pub fn validate_device_key(&self, key: &str) -> Option<(DeviceId, String)> {
        self.device_keys.get(key).map(|r| r.value().clone())
    }

    /// Register a connected device. Returns the per-session token; the WS
    /// handler must pass it back to `unregister_device_if_session` so a late
    /// cleanup from a dead socket can't tear down a fresh reconnect.
    pub async fn register_device(
        &self,
        id: DeviceId,
        name: String,
        scripts: Vec<ScriptInfo>,
        lan_ip: Option<IpAddr>,
        tx: mpsc::Sender<ServerMessage>,
    ) -> Uuid {
        let now = Utc::now();
        let session_id = Uuid::new_v4();

        // Carry forward any prior status snapshot so HomeKit doesn't blink to
        // "unknown" while we wait for the daemon's first status poll.
        let (prior_status, prior_observed_at, prior_lan_ip) = self
            .offline_devices
            .remove(&id)
            .map(|(_, d)| (d.last_status, d.status_observed_at, d.lan_ip))
            .unwrap_or_default();

        self.devices.insert(
            id,
            ConnectedDevice {
                id,
                name,
                connected_at: now,
                last_seen: now,
                status: prior_status,
                status_observed_at: prior_observed_at,
                scripts,
                // New daemon report wins; otherwise fall back to last-known.
                lan_ip: lan_ip.or(prior_lan_ip),
                tx,
                session_id,
            },
        );

        self.persist().await;
        self.emit_switch_events_for_device(&id);
        session_id
    }

    /// Unregister a device only if the current session matches the given
    /// token. Prevents a late TCP-close from a dead socket from kicking the
    /// fresh reconnect that took its place.
    pub async fn unregister_device_if_session(&self, id: &DeviceId, session_id: Uuid) {
        let matches = self
            .devices
            .get(id)
            .map(|d| d.session_id == session_id)
            .unwrap_or(false);
        if matches {
            if let Some((_, device)) = self.devices.remove(id) {
                self.offline_devices.insert(
                    *id,
                    OfflineDevice {
                        id: device.id,
                        name: device.name,
                        disconnected_at: Utc::now(),
                        last_status: device.status,
                        status_observed_at: device.status_observed_at,
                        scripts: device.scripts,
                        lan_ip: device.lan_ip,
                    },
                );
                self.persist().await;
                self.emit_switch_events_for_device(id);
            }
        }
    }

    /// Sweep devices we haven't heard from recently. Called periodically by
    /// a background task; ensures HomeKit reflects shutdown quickly even
    /// when the daemon's WS connection dies without a clean close (the
    /// common case after `systemctl poweroff`).
    pub async fn disconnect_stale_devices(&self, max_silence: std::time::Duration) {
        let cutoff = Utc::now()
            - chrono::Duration::from_std(max_silence)
                .unwrap_or_else(|_| chrono::Duration::seconds(30));
        let stale: Vec<(DeviceId, Uuid)> = self
            .devices
            .iter()
            .filter(|r| r.last_seen < cutoff)
            .map(|r| (r.id, r.session_id))
            .collect();
        for (id, sid) in stale {
            tracing::warn!(device_id = %id, "Disconnecting stale device (no traffic in window)");
            self.unregister_device_if_session(&id, sid).await;
        }
    }

    /// Update device status.
    pub async fn update_status(&self, id: &DeviceId, status: DeviceStatus) {
        let now = Utc::now();
        let changed = if let Some(mut device) = self.devices.get_mut(id) {
            device.status = status;
            device.last_seen = now;
            device.status_observed_at = Some(now);
            true
        } else {
            false
        };
        if changed {
            self.persist().await;
            self.emit_switch_events_for_device(id);
        }
    }

    /// Update device last seen.
    pub fn touch_device(&self, id: &DeviceId) {
        if let Some(mut device) = self.devices.get_mut(id) {
            device.last_seen = Utc::now();
        }
    }

    /// Get all devices as API response (online + recently offline).
    pub fn list_devices(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self
            .devices
            .iter()
            .map(|r| {
                let d = r.value();
                Device {
                    id: d.id,
                    name: d.name.clone(),
                    online: true,
                    last_seen: d.last_seen,
                    status: d.status.clone(),
                }
            })
            .collect();

        for r in self.offline_devices.iter() {
            let d = r.value();
            devices.push(Device {
                id: d.id,
                name: d.name.clone(),
                online: false,
                last_seen: d.disconnected_at,
                status: d.last_status.clone(),
            });
        }

        devices
    }

    /// Get a specific device (online or offline).
    pub fn get_device(&self, id: &DeviceId) -> Option<Device> {
        if let Some(r) = self.devices.get(id) {
            let d = r.value();
            return Some(Device {
                id: d.id,
                name: d.name.clone(),
                online: true,
                last_seen: d.last_seen,
                status: d.status.clone(),
            });
        }

        self.offline_devices.get(id).map(|r| {
            let d = r.value();
            Device {
                id: d.id,
                name: d.name.clone(),
                online: false,
                last_seen: d.disconnected_at,
                status: d.last_status.clone(),
            }
        })
    }

    /// Send a command to a device.
    pub async fn send_to_device(
        &self,
        id: &DeviceId,
        message: ServerMessage,
    ) -> Result<(), String> {
        match self.devices.get(id) {
            Some(device) => device
                .tx
                .send(message)
                .await
                .map_err(|_| "Failed to send to device".to_string()),
            None => Err("Device not connected".to_string()),
        }
    }

    /// Get scripts for a device (online or offline).
    pub fn get_device_scripts(&self, id: &DeviceId) -> Option<Vec<ScriptInfo>> {
        if let Some(d) = self.devices.get(id) {
            return Some(d.scripts.clone());
        }
        self.offline_devices.get(id).map(|d| d.scripts.clone())
    }

    // -----------------------------------------------------------------------
    // HomeKit
    // -----------------------------------------------------------------------

    /// All HomeKit switches configured for a specific device id.
    fn switches_for_device(&self, device_id: &DeviceId) -> Vec<HomeKitSwitchConfig> {
        let Some(homekit) = &self.config.homekit else {
            return Vec::new();
        };
        homekit
            .switches
            .iter()
            .filter(|s| {
                Uuid::parse_str(&s.device_id)
                    .map(|id| id == *device_id)
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    /// All HomeKit switches across all devices.
    pub fn all_switches(&self) -> Vec<HomeKitSwitchConfig> {
        self.config
            .homekit
            .as_ref()
            .map(|h| h.switches.clone())
            .unwrap_or_default()
    }

    /// Find a switch by user-facing id.
    pub fn find_switch(&self, switch_id: &str) -> Option<HomeKitSwitchConfig> {
        self.config
            .homekit
            .as_ref()?
            .switches
            .iter()
            .find(|s| s.id == switch_id)
            .cloned()
    }

    fn device_snapshot(&self, id: &DeviceId) -> Option<DeviceSnapshot> {
        if let Some(d) = self.devices.get(id) {
            return Some(DeviceSnapshot {
                name: d.name.clone(),
                online: true,
                status: d.status.clone(),
                status_observed_at: d.status_observed_at,
                lan_ip: d.lan_ip,
            });
        }
        self.offline_devices.get(id).map(|d| DeviceSnapshot {
            name: d.name.clone(),
            online: false,
            status: d.last_status.clone(),
            status_observed_at: d.status_observed_at,
            lan_ip: d.lan_ip,
        })
    }

    /// Look up the human-readable device name from the static device key
    /// table — used when the device has never connected and isn't in the
    /// online/offline caches.
    fn configured_device_name(&self, id: &DeviceId) -> Option<String> {
        self.configured_device(id).map(|d| d.name.clone())
    }

    fn configured_device(&self, id: &DeviceId) -> Option<&crate::config::DeviceKeyConfig> {
        let id_str = id.to_string();
        self.config
            .devices
            .iter()
            .find(|d| d.device_id.eq_ignore_ascii_case(&id_str))
    }

    /// Build a public service state view from current device data + config.
    /// Returns `None` only when the entry's `device_id` doesn't parse as a
    /// UUID — otherwise the service is always represented, even before the
    /// referenced daemon has ever connected, so HomeKit accessories stay
    /// pinned.
    pub fn build_switch_state(&self, cfg: &HomeKitSwitchConfig) -> Option<HomeKitSwitchState> {
        let device_id = Uuid::parse_str(&cfg.device_id).ok()?;

        // The accessory is the device. Every service configured for this
        // device_id ends up grouped under one HomeKit accessory.
        let accessory_uuid = homekit_device_accessory_uuid(&device_id);

        // Per-service identity for HAP. Folds state_source into the hash so
        // changing a Status switch to Online is a fresh service.
        let state_script_for_id = match cfg.state_source {
            SwitchStateSource::Status => cfg.state_script.as_deref().unwrap_or(""),
            SwitchStateSource::Online => "@online",
        };
        let service_subtype = homekit_service_subtype(
            &cfg.on_script,
            &cfg.off_script,
            state_script_for_id,
        );

        let (device_name, device_online, on, last_observed, lan_ip) =
            match self.device_snapshot(&device_id) {
                Some(snap) => {
                    let on = match cfg.state_source {
                        SwitchStateSource::Online => Some(snap.online),
                        SwitchStateSource::Status => {
                            derive_switch_on_from_config(&snap.status, cfg)
                        }
                    };
                    (
                        snap.name,
                        snap.online,
                        on,
                        snap.status_observed_at,
                        snap.lan_ip,
                    )
                }
                None => {
                    let name = self
                        .configured_device_name(&device_id)
                        .unwrap_or_else(|| device_id.to_string());
                    let on = match cfg.state_source {
                        // Never-seen device is by definition offline → OFF.
                        SwitchStateSource::Online => Some(false),
                        SwitchStateSource::Status => None,
                    };
                    (name, false, on, None, None)
                }
            };

        let wol_mac = self
            .configured_device(&device_id)
            .and_then(|d| d.wol_mac.clone());
        let wol_ip = lan_ip.map(|ip| ip.to_string());

        Some(HomeKitSwitchState {
            id: cfg.id.clone(),
            accessory_uuid,
            service_subtype,
            accessory_type: cfg.accessory_type,
            name: cfg.name.clone(),
            device_id,
            device_name,
            device_online,
            on,
            last_observed,
            wol_mac,
            wol_ip,
        })
    }

    /// Emit a SwitchUpdated event for every switch whose configured device
    /// matches `device_id`. No-op if no subscribers or no matching switches.
    fn emit_switch_events_for_device(&self, device_id: &DeviceId) {
        if self.homekit_events.receiver_count() == 0 {
            return;
        }
        for cfg in self.switches_for_device(device_id) {
            if let Some(state) = self.build_switch_state(&cfg) {
                let _ = self
                    .homekit_events
                    .send(HomeKitEvent::SwitchUpdated { state });
            }
        }
    }
}

/// Derive switch on/off using a switch's configured `state_script` and
/// `state_on_value`. Returns `None` if either is missing (config error for a
/// Status switch — the switch will read OFF in HomeKit, which is a visible
/// signal that something needs fixing).
fn derive_switch_on_from_config(
    status: &DeviceStatus,
    cfg: &HomeKitSwitchConfig,
) -> Option<bool> {
    let state_script = cfg.state_script.as_deref()?;
    let state_on_value = cfg.state_on_value.as_deref()?;
    derive_switch_on(status, state_script, state_on_value)
}

/// Derive switch on/off from a device status snapshot.
/// Returns `None` if the state script field is absent (never reported yet).
pub fn derive_switch_on(
    status: &DeviceStatus,
    state_script: &str,
    state_on_value: &str,
) -> Option<bool> {
    let raw = status.custom.get(state_script)?;
    let value_str = match raw {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => return None,
        other => other.to_string(),
    };
    Some(value_str.trim().eq_ignore_ascii_case(state_on_value.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AuthConfig, Config, DeviceKeyConfig, HomeKitConfig, ServerConfig, SwitchStateSource,
    };
    use serde_json::json;
    use std::collections::HashMap;

    fn status_with(field: &str, value: serde_json::Value) -> DeviceStatus {
        let mut custom = HashMap::new();
        custom.insert(field.to_string(), value);
        DeviceStatus {
            custom,
            ..Default::default()
        }
    }

    #[test]
    fn derive_string_match_case_insensitive() {
        let status = status_with("is_locked", json!("YES"));
        assert_eq!(
            derive_switch_on(&status, "is_locked", "yes"),
            Some(true),
            "string match should be case-insensitive"
        );
    }

    #[test]
    fn derive_string_no_match_is_off() {
        let status = status_with("is_locked", json!("no"));
        assert_eq!(derive_switch_on(&status, "is_locked", "yes"), Some(false));
    }

    #[test]
    fn derive_bool_matches_stringified() {
        let status = status_with("is_muted", json!(true));
        assert_eq!(derive_switch_on(&status, "is_muted", "true"), Some(true));
        let status = status_with("is_muted", json!(false));
        assert_eq!(derive_switch_on(&status, "is_muted", "true"), Some(false));
    }

    #[test]
    fn derive_number_matches_stringified() {
        let status = status_with("level", json!(1));
        assert_eq!(derive_switch_on(&status, "level", "1"), Some(true));
    }

    #[test]
    fn derive_missing_field_is_none() {
        let status = DeviceStatus::default();
        assert_eq!(derive_switch_on(&status, "is_locked", "yes"), None);
    }

    #[test]
    fn derive_null_field_is_none() {
        let status = status_with("is_locked", json!(null));
        assert_eq!(derive_switch_on(&status, "is_locked", "yes"), None);
    }

    #[test]
    fn derive_strips_whitespace() {
        let status = status_with("is_locked", json!("  yes  "));
        assert_eq!(derive_switch_on(&status, "is_locked", "yes"), Some(true));
    }

    // ---------------------------------------------------------------------
    // SwitchStateSource::Online — Power switch behaviour
    // ---------------------------------------------------------------------

    const TEST_DEVICE_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn online_switch_config() -> HomeKitSwitchConfig {
        HomeKitSwitchConfig {
            id: "power".to_string(),
            name: "Power".to_string(),
            device_id: TEST_DEVICE_ID.to_string(),
            accessory_type: HomeKitAccessoryType::Switch,
            on_script: "noop".to_string(),
            off_script: "shutdown".to_string(),
            state_source: SwitchStateSource::Online,
            state_script: None,
            state_on_value: None,
        }
    }

    fn state_with_online_switch() -> AppState {
        let cfg = Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                state_path: None,
            },
            auth: AuthConfig {
                jwt_secret: "test".to_string(),
                jwt_expiry_hours: 1,
                admin_username: "admin".to_string(),
                admin_password_hash: "x".to_string(),
            },
            devices: vec![DeviceKeyConfig {
                device_id: TEST_DEVICE_ID.to_string(),
                name: "Test Device".to_string(),
                api_key: "k".to_string(),
                wol_mac: Some("aa:bb:cc:dd:ee:ff".to_string()),
            }],
            homekit: Some(HomeKitConfig {
                api_key: "homekit-key".to_string(),
                switches: vec![online_switch_config()],
            }),
        };
        AppState::new(cfg, None)
    }

    #[test]
    fn online_switch_reads_off_when_never_seen() {
        let state = state_with_online_switch();
        let sw = state.build_switch_state(&online_switch_config()).unwrap();
        assert_eq!(sw.on, Some(false));
        assert!(!sw.device_online);
    }

    #[test]
    fn online_switch_reads_off_when_offline() {
        let state = state_with_online_switch();
        let device_id = Uuid::parse_str(TEST_DEVICE_ID).unwrap();
        let now = chrono::Utc::now();
        state.offline_devices.insert(
            device_id,
            OfflineDevice {
                id: device_id,
                name: "Test Device".to_string(),
                disconnected_at: now,
                last_status: DeviceStatus::default(),
                status_observed_at: Some(now),
                scripts: vec![],
                lan_ip: None,
            },
        );
        let sw = state.build_switch_state(&online_switch_config()).unwrap();
        assert_eq!(sw.on, Some(false));
        assert!(!sw.device_online);
    }

    #[test]
    fn online_switch_reads_on_when_connected() {
        let state = state_with_online_switch();
        let device_id = Uuid::parse_str(TEST_DEVICE_ID).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let now = chrono::Utc::now();
        state.devices.insert(
            device_id,
            ConnectedDevice {
                id: device_id,
                name: "Test Device".to_string(),
                connected_at: now,
                last_seen: now,
                status: DeviceStatus::default(),
                status_observed_at: None,
                scripts: vec![],
                lan_ip: None,
                tx,
                session_id: Uuid::new_v4(),
            },
        );
        let sw = state.build_switch_state(&online_switch_config()).unwrap();
        assert_eq!(sw.on, Some(true));
        assert!(sw.device_online);
    }

    #[test]
    fn services_on_same_device_share_accessory_uuid_but_distinct_subtypes() {
        let state = state_with_online_switch();

        let online_cfg = online_switch_config();
        let mut status_cfg = online_cfg.clone();
        status_cfg.state_source = SwitchStateSource::Status;
        status_cfg.state_script = Some("anything".to_string());
        status_cfg.state_on_value = Some("yes".to_string());

        let online_sw = state.build_switch_state(&online_cfg).unwrap();
        let status_sw = state.build_switch_state(&status_cfg).unwrap();
        assert_eq!(
            online_sw.accessory_uuid, status_sw.accessory_uuid,
            "Services on the same device must share an accessory UUID"
        );
        assert_ne!(
            online_sw.service_subtype, status_sw.service_subtype,
            "Distinct services on the same accessory must have distinct subtypes"
        );
    }

    #[test]
    fn lock_accessory_type_round_trips() {
        let state = state_with_online_switch();
        let mut cfg = online_switch_config();
        cfg.accessory_type = HomeKitAccessoryType::Lock;
        let sw = state.build_switch_state(&cfg).unwrap();
        assert_eq!(sw.accessory_type, HomeKitAccessoryType::Lock);
    }

    #[test]
    fn wol_mac_propagates_from_device_config_to_switch_state() {
        let state = state_with_online_switch();
        let sw = state.build_switch_state(&online_switch_config()).unwrap();
        assert_eq!(sw.wol_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }
}
