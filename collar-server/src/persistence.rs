//! Persist the device cache to disk so HomeKit accessories survive server restarts.
//!
//! Persists a snapshot of every device we know about — both currently online
//! and previously seen — as a single JSON blob. Online devices are persisted
//! using their most recent reported state, so a server restart followed by a
//! daemon reconnect doesn't lose the last-known status that HomeKit relies on.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use collar_common::{DeviceId, DeviceStatus, ScriptInfo};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// On-disk representation of a single device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedDevice {
    pub id: DeviceId,
    pub name: String,
    pub last_seen: DateTime<Utc>,
    pub last_status: DeviceStatus,
    pub scripts: Vec<ScriptInfo>,
    /// When the `last_status` was reported by the daemon. `None` if we've
    /// never received a status message for this device.
    #[serde(default)]
    pub status_observed_at: Option<DateTime<Utc>>,
    /// LAN IP the daemon reported at last connect — used for Wake-on-LAN
    /// targeting on networks that drop broadcasts. Stored as a string so
    /// the file stays human-editable.
    #[serde(default)]
    pub lan_ip: Option<String>,
}

/// Root of the persisted state file.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    /// Schema version — bump on breaking changes.
    #[serde(default = "default_version")]
    pub version: u32,
    pub devices: Vec<PersistedDevice>,
}

fn default_version() -> u32 {
    1
}

/// Writer that serializes saves so concurrent callers can't corrupt the file.
pub struct StatePersister {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl StatePersister {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load persisted state, returning an empty state if the file does not exist.
    pub fn load(&self) -> Result<PersistedState> {
        if !self.path.exists() {
            debug!(path = %self.path.display(), "No persisted state file yet");
            return Ok(PersistedState {
                version: default_version(),
                devices: Vec::new(),
            });
        }

        let content = std::fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read state file: {}", self.path.display()))?;
        let state: PersistedState = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse state file: {}", self.path.display()))?;
        Ok(state)
    }

    /// Atomically write the given state to disk. Uses tempfile + rename so a
    /// crash mid-write can't leave a truncated file.
    pub async fn save(&self, state: &PersistedState) -> Result<()> {
        let _guard = self.write_lock.lock().await;

        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));

        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("Failed to create state directory: {}", parent.display())
            })?;
        }

        let json = serde_json::to_vec_pretty(state).context("Failed to serialize state")?;

        let tmp_path = self.path.with_extension("json.tmp");
        let mut tmp = tokio::fs::File::create(&tmp_path)
            .await
            .with_context(|| format!("Failed to create temp file: {}", tmp_path.display()))?;
        tmp.write_all(&json).await?;
        tmp.sync_all().await?;
        drop(tmp);

        tokio::fs::rename(&tmp_path, &self.path)
            .await
            .with_context(|| {
                format!(
                    "Failed to rename {} -> {}",
                    tmp_path.display(),
                    self.path.display()
                )
            })?;

        debug!(path = %self.path.display(), devices = state.devices.len(), "Persisted state");
        Ok(())
    }

    /// Save best-effort: log on failure but don't propagate. Use this on hot
    /// paths (e.g. every status update) where a transient disk error shouldn't
    /// crash the server.
    pub async fn save_best_effort(&self, state: &PersistedState) {
        if let Err(e) = self.save(state).await {
            warn!(error = %e, "Failed to persist state");
        }
    }
}
