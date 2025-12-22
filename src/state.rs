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
        let path = PathBuf::from(&runtime_dir).join("richspace").join("state.json");
        tracing::trace!(
            path = %path.display(),
            runtime_dir = %runtime_dir,
            "resolved state file path"
        );
        path
    }

    /// Load state from ephemeral file
    pub fn load() -> Result<Self> {
        let start = std::time::Instant::now();
        let path = Self::state_path();

        tracing::debug!(
            path = %path.display(),
            exists = path.exists(),
            "loading ephemeral state"
        );

        let state = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    tracing::trace!(
                        path = %path.display(),
                        size_bytes = content.len(),
                        "read state file content"
                    );
                    match serde_json::from_str::<State>(&content) {
                        Ok(state) => {
                            tracing::info!(
                                path = %path.display(),
                                workspace_count = state.workspaces.len(),
                                version = state.version,
                                elapsed_us = start.elapsed().as_micros(),
                                "state loaded successfully"
                            );
                            state
                        }
                        Err(e) => {
                            tracing::error!(
                                path = %path.display(),
                                error = %e,
                                elapsed_us = start.elapsed().as_micros(),
                                "failed to parse state JSON"
                            );
                            return Err(e.into());
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        path = %path.display(),
                        error = %e,
                        elapsed_us = start.elapsed().as_micros(),
                        "failed to read state file"
                    );
                    return Err(e.into());
                }
            }
        } else {
            tracing::info!(
                path = %path.display(),
                elapsed_us = start.elapsed().as_micros(),
                "state file does not exist, using defaults"
            );
            State::default()
        };

        Ok(state)
    }

    /// Save state to ephemeral file
    pub fn save(&self) -> Result<()> {
        let start = std::time::Instant::now();
        let path = Self::state_path();

        tracing::debug!(
            path = %path.display(),
            workspace_count = self.workspaces.len(),
            version = self.version,
            "saving ephemeral state"
        );

        if let Some(parent) = path.parent() {
            if !parent.exists() {
                tracing::trace!(dir = %parent.display(), "creating parent directory");
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tracing::error!(
                        dir = %parent.display(),
                        error = %e,
                        elapsed_us = start.elapsed().as_micros(),
                        "failed to create parent directory"
                    );
                    return Err(e.into());
                }
            }
        }

        let content = match serde_json::to_string_pretty(self) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    elapsed_us = start.elapsed().as_micros(),
                    "failed to serialize state to JSON"
                );
                return Err(e.into());
            }
        };

        tracing::trace!(
            path = %path.display(),
            size_bytes = content.len(),
            "writing state to file"
        );

        if let Err(e) = std::fs::write(&path, &content) {
            tracing::error!(
                path = %path.display(),
                error = %e,
                elapsed_us = start.elapsed().as_micros(),
                "failed to write state file"
            );
            return Err(e.into());
        }

        tracing::info!(
            path = %path.display(),
            size_bytes = content.len(),
            workspace_count = self.workspaces.len(),
            version = self.version,
            elapsed_us = start.elapsed().as_micros(),
            "state saved successfully"
        );

        Ok(())
    }

    /// Get state for a specific workspace
    ///
    /// HOT PATH: Called ~7 times per workspace per render = 84+ calls/render
    /// No logging here - would flood at any render rate.
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
        tracing::debug!(
            workspace,
            label = ?label,
            version_before = self.version,
            "set_label mutation"
        );
        let entry = self.workspaces.entry(key).or_default();
        entry.label = label.clone();
        self.version += 1;
        tracing::trace!(
            workspace,
            version_after = self.version,
            "set_label complete"
        );
    }

    /// Set custom icon for a workspace
    pub fn set_icon(&mut self, workspace: i32, icon: Option<String>) {
        let key = workspace.to_string();
        tracing::debug!(
            workspace,
            icon = ?icon,
            version_before = self.version,
            "set_icon mutation"
        );
        let entry = self.workspaces.entry(key).or_default();
        entry.icon = icon.clone();
        self.version += 1;
        tracing::trace!(
            workspace,
            version_after = self.version,
            "set_icon complete"
        );
    }

    /// Clear state for a specific workspace (revert to defaults)
    pub fn clear(&mut self, workspace_num: i32) {
        tracing::debug!(
            workspace = workspace_num,
            version_before = self.version,
            "clear workspace mutation"
        );
        let removed = self.workspaces.remove(&workspace_num.to_string());
        self.version += 1;
        tracing::trace!(
            workspace = workspace_num,
            had_state = removed.is_some(),
            version_after = self.version,
            "clear complete"
        );
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
