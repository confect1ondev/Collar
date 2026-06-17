//! Shared types between collar-daemon and collar-server.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
