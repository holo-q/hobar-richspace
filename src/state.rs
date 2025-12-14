//! Ephemeral workspace state
//!
//! Stores per-workspace labels/icons that can be changed at runtime.
//! This is EPHEMERAL - stored in $XDG_RUNTIME_DIR and resets on logout.
//!
//! External tools (like babel) can write to this file to rename workspaces,
//! and the plugin will live-reload the changes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;

/// Per-workspace display state
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceState {
    /// Custom icon/label for this workspace (overrides config default)
    #[serde(default)]
    pub icon: Option<String>,

    /// Custom label text (displayed alongside or instead of icon, depending on panel config)
    /// External daemons can use this to show dynamic workspace context (e.g., "Building...")
    #[serde(default)]
    pub label: Option<String>,

    /// Custom tooltip text
    #[serde(default)]
    pub tooltip: Option<String>,

    /// CSS class to add to this workspace button
    #[serde(default)]
    pub css_class: Option<String>,

    /// Force urgency indicator (window attention state)
    /// External daemons can set this to true to programmatically trigger urgency styling
    #[serde(default)]
    pub urgent: Option<bool>,
}

/// Ephemeral state for all workspaces
///
/// Stored in $XDG_RUNTIME_DIR/richspace/state.json
/// Keyed by workspace number (as string for JSON compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct State {
    /// Per-workspace state, keyed by workspace number
    pub workspaces: HashMap<String, WorkspaceState>,

    /// Global state version (for debugging/tooling)
    #[serde(default)]
    pub version: u32,
}

#[allow(dead_code)]
impl State {
    /// Get the ephemeral state file path
    ///
    /// Returns $XDG_RUNTIME_DIR/richspace/state.json
    /// Falls back to /tmp/richspace/state.json if XDG_RUNTIME_DIR not set
    pub fn state_path() -> PathBuf {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime_dir).join("richspace").join("state.json")
    }

    /// Load state from ephemeral file
    pub fn load() -> Result<Self> {
        let path = Self::state_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let state: State = serde_json::from_str(&content)?;
            Ok(state)
        } else {
            Ok(State::default())
        }
    }

    /// Save state to ephemeral file
    pub fn save(&self) -> Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Get state for a specific workspace
    pub fn get(&self, workspace_num: i32) -> Option<&WorkspaceState> {
        self.workspaces.get(&workspace_num.to_string())
    }

    /// Set state for a specific workspace
    pub fn set(&mut self, workspace_num: i32, state: WorkspaceState) {
        self.workspaces.insert(workspace_num.to_string(), state);
        self.version += 1;
    }

    /// Set custom label for a workspace
    pub fn set_label(&mut self, workspace: i32, label: Option<String>) {
        let key = workspace.to_string();
        let entry = self.workspaces.entry(key).or_default();
        entry.label = label;
        self.version += 1;
    }

    /// Set custom icon for a workspace
    pub fn set_icon(&mut self, workspace: i32, icon: Option<String>) {
        let key = workspace.to_string();
        let entry = self.workspaces.entry(key).or_default();
        entry.icon = icon;
        self.version += 1;
    }

    /// Clear state for a specific workspace (revert to defaults)
    pub fn clear(&mut self, workspace_num: i32) {
        self.workspaces.remove(&workspace_num.to_string());
        self.version += 1;
    }

    /// Clear all workspace states
    pub fn clear_all(&mut self) {
        self.workspaces.clear();
        self.version += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_path() {
        let path = State::state_path();
        assert!(path.to_string_lossy().contains("richspace"));
        assert!(path.to_string_lossy().ends_with("state.json"));
    }

    #[test]
    fn test_workspace_state() {
        let mut state = State::default();
        state.set(0, WorkspaceState {
            icon: Some("🏠".to_string()),
            label: Some("Home".to_string()),
            tooltip: Some("Home workspace".to_string()),
            css_class: None,
            urgent: Some(false),
        });

        assert_eq!(state.get(0).unwrap().icon, Some("🏠".to_string()));
        assert_eq!(state.get(0).unwrap().label, Some("Home".to_string()));
        assert_eq!(state.get(0).unwrap().urgent, Some(false));
        assert!(state.get(1).is_none());
    }

    #[test]
    fn test_backward_compat() {
        // Ensure old state files (without label/urgent) deserialize correctly
        let old_json = r#"{
            "workspaces": {
                "0": {
                    "icon": "🏠",
                    "tooltip": "Home"
                }
            },
            "version": 1
        }"#;

        let state: State = serde_json::from_str(old_json).unwrap();
        let ws = state.get(0).unwrap();

        assert_eq!(ws.icon, Some("🏠".to_string()));
        assert_eq!(ws.label, None);  // Missing fields default to None
        assert_eq!(ws.urgent, None);
    }
}
