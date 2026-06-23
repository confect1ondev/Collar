//! Shared types between collar-daemon and collar-server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Namespace UUID for deriving stable HomeKit accessory identities.
/// Hard-coded; treat as part of the wire contract.
pub const HOMEKIT_NAMESPACE: Uuid = Uuid::from_bytes([
    0xc0, 0x11, 0xa7, 0x5e, 0x9a, 0xfe, 0x40, 0x1d, 0xb0, 0x55, 0xc0, 0x11, 0xa7, 0x5e, 0xb0, 0x55,
]);

/// Derive the stable HomeKit accessory UUID for an entire device. All
/// services (Switch, Lock, …) configured for a given device live under this
/// one accessory in HomeKit, so identity is device-level.
pub fn homekit_device_accessory_uuid(device_id: &Uuid) -> Uuid {
    let mut name = String::with_capacity(64);
    name.push_str("device|");
    name.push_str(&device_id.to_string());
    Uuid::new_v5(&HOMEKIT_NAMESPACE, name.as_bytes())
}

/// Derive a stable subtype string for one service on a device's accessory.
/// HAP requires this when multiple services of the same type share an
/// accessory. Derived from the *behaviour* (on/off/state) so that renaming
/// the user-facing `id` in config is a free operation.
pub fn homekit_service_subtype(on_script: &str, off_script: &str, state_script: &str) -> String {
    let mut name = String::with_capacity(96);
    name.push_str("svc|");
    name.push_str(on_script);
    name.push('|');
    name.push_str(off_script);
    name.push('|');
    name.push_str(state_script);
    Uuid::new_v5(&HOMEKIT_NAMESPACE, name.as_bytes()).to_string()
}

/// Unique device identifier.
pub type DeviceId = Uuid;

/// Unique command identifier.
pub type CommandId = Uuid;

/// Script identifier (e.g., "lock", "unlock", "is_locked").
pub type ScriptId = String;

// ============================================================================
// WebSocket Messages
// ============================================================================

/// Messages sent from daemon to server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonMessage {
    /// Initial authentication with available scripts.
    Auth {
        device_key: String,
        #[serde(default)]
        scripts: Vec<ScriptInfo>,
        /// LAN IPv4 address the daemon currently has on its primary route.
        /// Optional for backward compatibility — older daemons may omit it.
        /// Used server-side for Wake-on-LAN unicast targeting.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        lan_ip: Option<String>,
    },

    /// Result of command execution.
    CommandResult {
        command_id: CommandId,
        success: bool,
        output: Option<String>,
        error: Option<String>,
    },

    /// Periodic status update.
    Status { data: DeviceStatus },

    /// Heartbeat/ping.
    Ping,
}

/// Messages sent from server to daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Authentication response.
    AuthResult { success: bool, error: Option<String> },

    /// Execute a script.
    Execute {
        command_id: CommandId,
        script_id: ScriptId,
        args: Option<Vec<String>>,
    },

    /// Request immediate status update.
    RequestStatus,

    /// Heartbeat/pong.
    Pong,
}

// ============================================================================
// Device & Status
// ============================================================================

/// Current device status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceStatus {
    /// Whether the screen is locked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,

    /// Battery percentage (0-100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<u8>,

    /// Whether on AC power.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charging: Option<bool>,

    /// Current volume (0-100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,

    /// Whether audio is muted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,

    /// Custom status fields.
    #[serde(flatten)]
    pub custom: std::collections::HashMap<String, serde_json::Value>,
}

/// Device information as seen by the server/frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub online: bool,
    pub last_seen: DateTime<Utc>,
    pub status: DeviceStatus,
}

// ============================================================================
// Scripts
// ============================================================================

/// Type of script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    /// Performs an action (e.g., lock screen).
    Action,
    /// Returns status information (e.g., is locked?).
    Status,
}

/// Script definition (internal, with command).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub id: ScriptId,
    pub name: String,
    pub description: String,
    #[serde(alias = "type")]
    pub script_type: ScriptType,
    /// Shell command to execute.
    pub command: String,
    /// Optional icon name for frontend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Script info for API/frontend (no command exposed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptInfo {
    pub id: ScriptId,
    pub name: String,
    pub description: String,
    pub script_type: ScriptType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl From<&Script> for ScriptInfo {
    fn from(s: &Script) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            script_type: s.script_type,
            icon: s.icon.clone(),
        }
    }
}

// ============================================================================
// API Types
// ============================================================================

/// Request to execute a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    pub script_id: ScriptId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

/// Response from command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    pub command_id: CommandId,
}

/// Login request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Login response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

// ============================================================================
// HomeKit
// ============================================================================

/// Which HomeKit service type a configured switch is exposed as.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HomeKitAccessoryType {
    /// Standard on/off toggle. Default.
    #[default]
    Switch,
    /// HomeKit LockMechanism — surfaces as a lock in the Home app and via
    /// Siri ("lock my desktop"). `on=true` means Secured (locked).
    Lock,
}

/// State of one service on a device's HomeKit accessory.
///
/// Multiple of these can share an `accessory_uuid` — that's how Power and
/// Lock end up grouped under a single "Blue Desktop" accessory in HomeKit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeKitSwitchState {
    /// User-facing service identifier (matches `[[homekit.switches]].id`).
    /// Cosmetic; renaming it does not re-pair anything.
    pub id: String,
    /// Stable HomeKit accessory identity. Shared across every service on
    /// the same device — i.e. one accessory per `device_id`.
    pub accessory_uuid: Uuid,
    /// Stable per-service identifier within the accessory. HAP requires
    /// this when multiple services of the same type live on one accessory.
    pub service_subtype: String,
    /// HomeKit service type this exposes (Switch or LockMechanism).
    #[serde(default)]
    pub accessory_type: HomeKitAccessoryType,
    /// Display name shown for this *service* in the Home app.
    pub name: String,
    pub device_id: DeviceId,
    /// Display name shown for the device-level accessory in the Home app.
    pub device_name: String,
    /// Whether the underlying daemon is currently connected.
    pub device_online: bool,
    /// Current on/off state derived from the device's last status report.
    /// For `Lock`, `true` == Secured. `None` if state has never been
    /// observed (still appears in HomeKit as OFF/Unsecured initially).
    pub on: Option<bool>,
    /// ISO timestamp of the most recent status observation from the daemon.
    /// `None` if the daemon has never reported status for this device.
    pub last_observed: Option<DateTime<Utc>>,
    /// MAC address (colon- or dash-separated, or unseparated hex) for the
    /// device's wired NIC. When set and the device is currently offline,
    /// the Homebridge plugin will fire a Wake-on-LAN magic packet on
    /// SET on=true instead of asking the server (which can't reach an
    /// offline daemon anyway).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub wol_mac: Option<String>,
    /// LAN IPv4 of the device's NIC, last reported by the daemon on
    /// connection. Used by the plugin for **unicast** WoL on networks that
    /// drop broadcasts (e.g. mesh routers). Stale if the device has been
    /// offline long enough for the router's ARP cache to expire.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub wol_ip: Option<String>,
}

/// Request body for setting a switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeKitSetRequest {
    pub on: bool,
}

/// Response when toggling a switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeKitSetResponse {
    pub command_id: CommandId,
    /// The script id that was dispatched.
    pub dispatched_script: ScriptId,
}

/// Events streamed to subscribed HomeKit clients over SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HomeKitEvent {
    /// A switch's state, online status, or display name has changed.
    SwitchUpdated { state: HomeKitSwitchState },
    /// Server-side keepalive so clients can detect dead connections.
    Heartbeat,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum CollarError {
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(DeviceId),

    #[error("Device offline: {0}")]
    DeviceOffline(DeviceId),

    #[error("Script not found: {0}")]
    ScriptNotFound(ScriptId),

    #[error("Command execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Connection error: {0}")]
    Connection(String),
}
