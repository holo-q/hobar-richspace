//! Persistent configuration
//!
//! Stores plugin settings like default icons, styling preferences.
//! This is PERSISTENT across sessions (stored in ~/.config/xfce4/panel/).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use anyhow::Result;

/// How to display each workspace button
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    IconOnly,
    LabelOnly,
    #[default]
    IconAndLabel,
}

/// Source for workspace labels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// Show workspace number (1, 2, 3...)
    #[default]
    Number,
    /// Show window manager's workspace name
    WmName,
    /// Show custom label from state file only
    Custom,
}

/// How to display window count per workspace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowCountDisplay {
    /// Don't show window count
    Hidden,
    /// Show in tooltip only
    #[default]
    Tooltip,
    /// Show as badge/superscript
    Badge,
    /// Show inline with label
    Inline,
}

/// Persistent plugin configuration
///
/// Stored in ~/.config/xfce4/panel/richspace-N.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ─── Legacy Fields (kept for backward compatibility) ────────────────────
    /// Default icon/label for workspaces without custom state
    /// Can be emoji, nerd font icon, or text
    pub default_icon: String,

    /// Icon/label for active workspace (if different from default)
    pub active_icon: Option<String>,

    /// Whether to show workspace numbers alongside icons
    pub show_numbers: bool,

    /// Whether to show workspace names (from WM) as tooltips
    pub show_name_tooltips: bool,

    /// Whether to show window count indicators
    pub show_window_count: bool,

    /// Spacing between workspace buttons (pixels)
    pub spacing: i32,

    /// Minimum button width (pixels, 0 = auto)
    pub min_button_width: i32,

    /// CSS class prefix for styling
    pub css_class: String,

    /// Custom CSS (optional)
    pub custom_css: Option<String>,

    // ─── Display Settings ───────────────────────────────────────────────────
    /// How to display workspace buttons (icon, label, or both)
    #[serde(default)]
    pub display_mode: DisplayMode,

    /// Where workspace labels come from
    #[serde(default)]
    pub label_source: LabelSource,

    /// Show label only for the active workspace
    #[serde(default)]
    pub active_only_label: bool,

    // ─── Icons ──────────────────────────────────────────────────────────────
    /// Icon for workspaces with no windows (None = use default_icon)
    #[serde(default)]
    pub empty_icon: Option<String>,

    // ─── Window Count ───────────────────────────────────────────────────────
    /// How to display window count per workspace
    #[serde(default)]
    pub window_count_display: WindowCountDisplay,

    // ─── Empty Workspaces ───────────────────────────────────────────────────
    /// Whether to show workspaces with no windows
    #[serde(default = "default_true")]
    pub show_empty_workspaces: bool,

    // ─── Scrolling ──────────────────────────────────────────────────────────
    /// Enable scroll wheel to switch workspaces
    #[serde(default = "default_true")]
    pub scroll_enabled: bool,

    /// Wrap to first/last workspace when scrolling past edges
    #[serde(default = "default_true")]
    pub scroll_wrap: bool,

    // ─── Typography ─────────────────────────────────────────────────────────
    /// Custom font family (None = use system default)
    #[serde(default)]
    pub font_family: Option<String>,

    /// Custom font size in points (None = use system default)
    #[serde(default)]
    pub font_size: Option<f32>,

    /// Custom font weight: "normal", "bold", "100"-"900" (None = use system default)
    #[serde(default)]
    pub font_weight: Option<String>,

    // ─── Layout ─────────────────────────────────────────────────────────────
    /// Padding inside each button (pixels)
    #[serde(default = "default_button_padding")]
    pub button_padding: i32,

    /// Maximum button width (None = unlimited)
    #[serde(default)]
    pub max_button_width: Option<i32>,

    /// Whether buttons should expand to fill available space
    #[serde(default)]
    pub expand_buttons: bool,
}

// Helper functions for serde defaults
fn default_true() -> bool {
    true
}

fn default_button_padding() -> i32 {
    4
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // Legacy fields - kept for backward compatibility
            default_icon: "○".to_string(),
            active_icon: Some("●".to_string()),
            show_numbers: false,
            show_name_tooltips: true,
            show_window_count: false,
            spacing: 2,
            min_button_width: 0,
            css_class: "richspace".to_string(),
            custom_css: None,

            // Display settings
            display_mode: DisplayMode::IconAndLabel,
            label_source: LabelSource::Number,
            active_only_label: false,

            // Icons
            empty_icon: Some("·".to_string()),

            // Window count
            window_count_display: WindowCountDisplay::Tooltip,

            // Empty workspaces
            show_empty_workspaces: true,

            // Scrolling
            scroll_enabled: true,
            scroll_wrap: true,

            // Typography - use system defaults
            font_family: None,
            font_size: None,
            font_weight: None,

            // Layout
            button_padding: 4,
            max_button_width: None,
            expand_buttons: false,
        }
    }
}

impl Config {
    /// Load config from JSON file
    pub fn load(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let config: Config = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    /// Save config to JSON file
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
