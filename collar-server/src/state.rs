//! Application state and device management.

use chrono::{DateTime, Utc};
use collar_common::{Device, DeviceId, DeviceStatus, ScriptInfo, ServerMessage};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::Config;

/// Connected device with its sender channel.
pub struct ConnectedDevice {
    pub id: DeviceId,
    pub name: String,
    pub connected_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub status: DeviceStatus,
    pub scripts: Vec<ScriptInfo>,
    pub tx: mpsc::Sender<ServerMessage>,
}

/// Cached info for recently disconnected devices.
pub struct OfflineDevice {
    pub id: DeviceId,
    pub name: String,
    pub disconnected_at: DateTime<Utc>,
    pub last_status: DeviceStatus,
    pub scripts: Vec<ScriptInfo>,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub devices: Arc<DashMap<DeviceId, ConnectedDevice>>,
    pub offline_devices: Arc<DashMap<DeviceId, OfflineDevice>>,
    pub device_keys: Arc<DashMap<String, (DeviceId, String)>>, // api_key -> (device_id, name)
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let device_keys = DashMap::new();

        // Load device keys from config
        for dk in &config.devices {
            let device_id = Uuid::parse_str(&dk.device_id)
                .unwrap_or_else(|_| Uuid::new_v4());
            device_keys.insert(dk.api_key.clone(), (device_id, dk.name.clone()));
        }

        Self {
            config: Arc::new(config),
            devices: Arc::new(DashMap::new()),
            offline_devices: Arc::new(DashMap::new()),
            device_keys: Arc::new(device_keys),
        }
    }

    /// Validate a device API key and return device info.
    pub fn validate_device_key(&self, key: &str) -> Option<(DeviceId, String)> {
        self.device_keys.get(key).map(|r| r.value().clone())
    }

    /// Register a connected device.
    pub fn register_device(
        &self,
        id: DeviceId,
        name: String,
        scripts: Vec<ScriptInfo>,
        tx: mpsc::Sender<ServerMessage>,
    ) {
        let now = Utc::now();

        // Remove from offline cache if present
        self.offline_devices.remove(&id);

        self.devices.insert(
            id,
            ConnectedDevice {
                id,
                name,
                connected_at: now,
                last_seen: now,
                status: DeviceStatus::default(),
                scripts,
                tx,
            },
        );
    }

    /// Unregister a device and cache its offline state.
    pub fn unregister_device(&self, id: &DeviceId) {
        if let Some((_, device)) = self.devices.remove(id) {
            // Cache the device as offline
            self.offline_devices.insert(
                *id,
                OfflineDevice {
                    id: device.id,
                    name: device.name,
                    disconnected_at: Utc::now(),
                    last_status: device.status,
                    scripts: device.scripts,
                },
            );
        }
    }

    /// Update device status.
    pub fn update_status(&self, id: &DeviceId, status: DeviceStatus) {
        if let Some(mut device) = self.devices.get_mut(id) {
            device.status = status;
            device.last_seen = Utc::now();
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
        let mut devices: Vec<Device> = self.devices
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

        // Add offline devices
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
        // Check online first
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

        // Check offline cache
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
            Some(device) => {
                device
                    .tx
                    .send(message)
                    .await
                    .map_err(|_| "Failed to send to device".to_string())
            }
            None => Err("Device not connected".to_string()),
        }
    }

    /// Get scripts for a device (online or offline).
    pub fn get_device_scripts(&self, id: &DeviceId) -> Option<Vec<ScriptInfo>> {
        // Check online first
        if let Some(d) = self.devices.get(id) {
            return Some(d.scripts.clone());
        }

        // Check offline cache
        self.offline_devices.get(id).map(|d| d.scripts.clone())
    }
}
