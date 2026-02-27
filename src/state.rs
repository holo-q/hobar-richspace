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

    /// Virtual display order — maps visual position to X11 workspace number.
    /// Empty or wrong length = fall back to identity order (0, 1, 2, ...).
    /// Reconciled on workspace add/remove via reconcile_display_order().
    #[serde(default)]
    pub display_order: Vec<i32>,
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

    /// Swap ephemeral state (labels, icons, CSS classes, urgent) between two workspaces.
    ///
    /// Used by true_reorder: when windows move between workspaces, their customizations
    /// should follow. After this call, workspace A has B's labels/icons and vice versa.
    pub fn swap_ephemeral(&mut self, ws_a: i32, ws_b: i32) {
        if ws_a == ws_b {
            return;
        }
        let key_a = ws_a.to_string();
        let key_b = ws_b.to_string();

        let state_a = self.workspaces.remove(&key_a);
        let state_b = self.workspaces.remove(&key_b);

        tracing::debug!(
            ws_a, ws_b,
            a_had_state = state_a.is_some(),
            b_had_state = state_b.is_some(),
            "swap_ephemeral"
        );

        // Cross-insert: A's state goes to B's key, B's state goes to A's key
        if let Some(s) = state_a {
            self.workspaces.insert(key_b, s);
        }
        if let Some(s) = state_b {
            self.workspaces.insert(key_a, s);
        }

        self.version += 1;
    }

    /// Get effective display order, falling back to identity if empty or stale.
    /// Always returns exactly `workspace_count` entries.
    pub fn effective_display_order(&self, workspace_count: usize) -> Vec<i32> {
        if self.display_order.len() == workspace_count {
            self.display_order.clone()
        } else {
            (0..workspace_count as i32).collect()
        }
    }

    /// Move workspace from display position `from` to display position `to`.
    /// Shifts other workspaces to accommodate (insert-at semantics, not swap).
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from < self.display_order.len() && to < self.display_order.len() && from != to {
            let ws = self.display_order.remove(from);
            self.display_order.insert(to, ws);
            self.version += 1;
            tracing::debug!(
                from,
                to,
                workspace = ws,
                new_order = ?self.display_order,
                version = self.version,
                "display order reorder"
            );
        }
    }

    /// Reconcile display order when workspace count changes.
    /// - Removed workspaces: dropped from order, others preserved in relative position
    /// - Added workspaces: appended at end
    /// - If display_order was empty, initializes to identity
    pub fn reconcile_display_order(&mut self, workspace_numbers: &[i32]) {
        let ws_set: std::collections::HashSet<i32> = workspace_numbers.iter().copied().collect();

        if self.display_order.is_empty() {
            // Initialize to identity
            self.display_order = workspace_numbers.to_vec();
            self.version += 1;
            tracing::debug!(
                order = ?self.display_order,
                "display order initialized to identity"
            );
            return;
        }

        let before = self.display_order.len();

        // Remove workspaces that no longer exist
        self.display_order.retain(|n| ws_set.contains(n));

        // Append new workspaces not already in order
        let existing: std::collections::HashSet<i32> = self.display_order.iter().copied().collect();
        for &ws in workspace_numbers {
            if !existing.contains(&ws) {
                self.display_order.push(ws);
            }
        }

        let after = self.display_order.len();
        if before != after {
            self.version += 1;
            tracing::info!(
                before_count = before,
                after_count = after,
                order = ?self.display_order,
                "display order reconciled"
            );
        }
    }

    /// Find the display position of a workspace number.
    /// Returns None if workspace is not in the display order.
    pub fn display_position_of(&self, workspace_num: i32) -> Option<usize> {
        self.display_order.iter().position(|&n| n == workspace_num)
    }

    /// Swap two adjacent display positions. Used for keyboard reorder.
    /// Returns the new display position of the moved workspace, or None if invalid.
    pub fn swap_adjacent(&mut self, pos: usize, direction: i32) -> Option<usize> {
        let new_pos = (pos as i32 + direction) as usize;
        if new_pos < self.display_order.len() {
            self.display_order.swap(pos, new_pos);
            self.version += 1;
            tracing::debug!(
                pos,
                new_pos,
                order = ?self.display_order,
                "display order swap"
            );
            Some(new_pos)
        } else {
            None
        }
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

    #[test]
    fn test_effective_display_order_identity() {
        let state = State::default();
        assert_eq!(state.effective_display_order(4), vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_effective_display_order_custom() {
        let mut state = State::default();
        state.display_order = vec![3, 1, 0, 2];
        assert_eq!(state.effective_display_order(4), vec![3, 1, 0, 2]);
    }

    #[test]
    fn test_effective_display_order_stale() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2]; // 3 entries but 4 workspaces
        assert_eq!(state.effective_display_order(4), vec![0, 1, 2, 3]); // Falls back to identity
    }

    #[test]
    fn test_reorder_move_to_front() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2, 3];
        state.reorder(2, 0);
        assert_eq!(state.display_order, vec![2, 0, 1, 3]);
    }

    #[test]
    fn test_reorder_move_to_back() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2, 3];
        state.reorder(0, 3);
        assert_eq!(state.display_order, vec![1, 2, 3, 0]);
    }

    #[test]
    fn test_reorder_noop() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2, 3];
        let v_before = state.version;
        state.reorder(2, 2);
        assert_eq!(state.version, v_before); // No version bump
        assert_eq!(state.display_order, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_reconcile_add_workspace() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2];
        state.reconcile_display_order(&[0, 1, 2, 3]);
        assert_eq!(state.display_order, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_reconcile_remove_workspace() {
        let mut state = State::default();
        state.display_order = vec![2, 0, 1, 3];
        state.reconcile_display_order(&[0, 1, 3]);
        assert_eq!(state.display_order, vec![0, 1, 3]);
    }

    #[test]
    fn test_reconcile_preserves_custom_order() {
        let mut state = State::default();
        state.display_order = vec![3, 1, 0, 2];
        state.reconcile_display_order(&[0, 1, 2, 3, 4]);
        assert_eq!(state.display_order, vec![3, 1, 0, 2, 4]);
    }

    #[test]
    fn test_reconcile_empty_initializes() {
        let mut state = State::default();
        state.reconcile_display_order(&[0, 1, 2]);
        assert_eq!(state.display_order, vec![0, 1, 2]);
    }

    #[test]
    fn test_swap_adjacent_right() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2, 3];
        let new_pos = state.swap_adjacent(1, 1);
        assert_eq!(new_pos, Some(2));
        assert_eq!(state.display_order, vec![0, 2, 1, 3]);
    }

    #[test]
    fn test_swap_adjacent_left() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2, 3];
        let new_pos = state.swap_adjacent(2, -1);
        assert_eq!(new_pos, Some(1));
        assert_eq!(state.display_order, vec![0, 2, 1, 3]);
    }

    #[test]
    fn test_swap_adjacent_boundary() {
        let mut state = State::default();
        state.display_order = vec![0, 1, 2, 3];
        assert_eq!(state.swap_adjacent(3, 1), None); // Can't go right from last
        assert_eq!(state.swap_adjacent(0, -1), None); // Can't go left from first (wraps to large usize)
    }

    #[test]
    fn test_display_position_of() {
        let mut state = State::default();
        state.display_order = vec![3, 1, 0, 2];
        assert_eq!(state.display_position_of(3), Some(0));
        assert_eq!(state.display_position_of(0), Some(2));
        assert_eq!(state.display_position_of(5), None);
    }

    #[test]
    fn test_swap_ephemeral_both_have_state() {
        let mut state = State::default();
        state.set(0, WorkspaceState {
            label: Some("Home".to_string()),
            icon: Some("H".to_string()),
            ..Default::default()
        });
        state.set(1, WorkspaceState {
            label: Some("Code".to_string()),
            icon: Some("C".to_string()),
            ..Default::default()
        });
        let v_before = state.version;
        state.swap_ephemeral(0, 1);
        assert_eq!(state.get(0).unwrap().label, Some("Code".to_string()));
        assert_eq!(state.get(0).unwrap().icon, Some("C".to_string()));
        assert_eq!(state.get(1).unwrap().label, Some("Home".to_string()));
        assert_eq!(state.get(1).unwrap().icon, Some("H".to_string()));
        assert!(state.version > v_before);
    }

    #[test]
    fn test_swap_ephemeral_one_empty() {
        let mut state = State::default();
        state.set(0, WorkspaceState {
            label: Some("Home".to_string()),
            ..Default::default()
        });
        // Workspace 1 has no state
        state.swap_ephemeral(0, 1);
        assert!(state.get(0).is_none()); // Was empty, stays empty (no entry)
        assert_eq!(state.get(1).unwrap().label, Some("Home".to_string()));
    }

    #[test]
    fn test_swap_ephemeral_noop_same() {
        let mut state = State::default();
        state.set(0, WorkspaceState {
            label: Some("Home".to_string()),
            ..Default::default()
        });
        let v_before = state.version;
        state.swap_ephemeral(0, 0);
        assert_eq!(state.version, v_before); // No version bump for noop
    }
}
