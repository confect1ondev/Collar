//! Script registry and management.

use collar_common::{Script, ScriptId, ScriptType};
use std::collections::HashMap;

/// Registry of available scripts.
#[derive(Debug, Default)]
pub struct ScriptRegistry {
    scripts: HashMap<ScriptId, Script>,
}

impl ScriptRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a script.
    pub fn register(&mut self, script: Script) {
        self.scripts.insert(script.id.clone(), script);
    }

    /// Get a script by ID.
    pub fn get(&self, id: &str) -> Option<&Script> {
        self.scripts.get(id)
    }

    /// Get all scripts.
    pub fn all(&self) -> impl Iterator<Item = &Script> {
        self.scripts.values()
    }

    /// Get all action scripts.
    pub fn actions(&self) -> impl Iterator<Item = &Script> {
        self.scripts
            .values()
            .filter(|s| s.script_type == ScriptType::Action)
    }

    /// Get all status scripts.
    pub fn status_scripts(&self) -> impl Iterator<Item = &Script> {
        self.scripts
            .values()
            .filter(|s| s.script_type == ScriptType::Status)
    }
}
