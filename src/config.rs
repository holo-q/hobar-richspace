//! Persistent configuration
//!
//! Stores plugin settings like default icons, styling preferences.
//! This is PERSISTENT across sessions (stored in ~/.config/xfce4/panel/).
//!
//! Config format is TOML with macro support for icon rules:
//! ```toml
//! [macros]
//! browser = ["firefox", "brave-browser", "chromium"]
//! fm = ["nemo", "nautilus", "thunar"]
//!
//! [[icon_rules]]
//! macro = "browser"  # References macros.browser
//! icon = "󰖟"
//! match_mode = "all"
//! ```

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// How windows must match for an icon rule to trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconMatchMode {
    /// ALL windows on workspace must match the pattern
    #[default]
    All,
    /// ANY window on workspace matches (at least one)
    Any,
}

/// Rule for automatically setting workspace icon based on window classes
///
/// Evaluated in order on each window change. First matching rule wins.
/// Uses WM_CLASS (class_group) for matching - e.g., "firefox", "kitty", "Code".
///
/// Rules can use either:
/// - `macro`: reference a predefined class list from [macros] section
/// - `class_regex`: raw regex pattern for custom matching
///
/// # Example (TOML)
/// ```toml
/// [[icon_rules]]
/// macro = "browser"      # Uses predefined macro
/// icon = "󰖟"
/// match_mode = "all"
///
/// [[icon_rules]]
/// class_regex = "^code$" # Raw regex
/// icon = "󰨞"
/// match_mode = "any"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconRule {
    /// Reference to a macro name (from [macros] section)
    /// Expands to regex: ^(class1|class2|...)$
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#macro: Option<String>,

    /// Regex pattern to match against WM_CLASS (case-insensitive)
    /// Used if `macro` is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_regex: Option<String>,

    /// Icon to display when rule matches (emoji, nerd font glyph, text)
    pub icon: String,

    /// How windows must match: "all" or "any"
    #[serde(default)]
    pub match_mode: IconMatchMode,

    /// Optional human-readable name for this rule
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl IconRule {
    /// Build the effective regex pattern, expanding macros if needed
    fn effective_pattern(&self, macros: &HashMap<String, Vec<String>>) -> Option<String> {
        if let Some(ref macro_name) = self.r#macro {
            // Expand macro to regex: ^(class1|class2|...)$
            if let Some(classes) = macros.get(macro_name) {
                if classes.is_empty() {
                    tracing::warn!(macro_name, "Empty macro referenced in icon rule");
                    return None;
                }
                // Escape regex special chars in class names, join with |
                let escaped: Vec<String> = classes.iter()
                    .map(|c| regex::escape(c))
                    .collect();
                Some(format!("^({})$", escaped.join("|")))
            } else {
                tracing::warn!(macro_name, "Unknown macro referenced in icon rule");
                None
            }
        } else {
            self.class_regex.clone()
        }
    }

    /// Compile the regex pattern (case-insensitive), expanding macros
    pub fn compile_regex(&self, macros: &HashMap<String, Vec<String>>) -> Option<Regex> {
        let pattern = self.effective_pattern(macros)?;
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
            .map_err(|e| tracing::warn!(pattern, error = %e, "Invalid icon rule regex"))
            .ok()
    }

    /// Check if a set of window classes matches this rule
    ///
    /// Returns true if the rule matches based on match_mode:
    /// - All: every class must match the pattern
    /// - Any: at least one class must match
    pub fn matches(&self, classes: &[String], macros: &HashMap<String, Vec<String>>) -> bool {
        if classes.is_empty() {
            tracing::trace!(
                rule_name = ?self.name,
                rule_icon = %self.icon,
                "no classes to match, returning false"
            );
            return false;
        }

        let Some(regex) = self.compile_regex(macros) else {
            tracing::trace!(
                rule_name = ?self.name,
                rule_icon = %self.icon,
                macro_name = ?self.r#macro,
                class_regex = ?self.class_regex,
                "failed to compile regex, returning false"
            );
            return false;
        };

        let result = match self.match_mode {
            IconMatchMode::All => classes.iter().all(|c| regex.is_match(c)),
            IconMatchMode::Any => classes.iter().any(|c| regex.is_match(c)),
        };

        tracing::trace!(
            rule_name = ?self.name,
            rule_icon = %self.icon,
            match_mode = ?self.match_mode,
            macro_name = ?self.r#macro,
            class_regex = ?self.class_regex,
            classes = ?classes,
            matched = result,
            "icon rule match evaluation"
        );

        result
    }
}

/// Persistent plugin configuration
///
/// Stored in ~/.config/xfce4/panel/richspace-N.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ─── Macros ────────────────────────────────────────────────────────────
    /// Predefined class patterns for icon rules
    /// Keys are macro names, values are lists of WM_CLASS strings
    /// Referenced in icon_rules via `macro = "name"`
    #[serde(default = "default_macros")]
    pub macros: HashMap<String, Vec<String>>,

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

    // ─── Icon Rules ────────────────────────────────────────────────────────
    /// Auto-icon rules based on window classes
    ///
    /// Evaluated in order on each window add/remove. First match wins.
    /// If no rule matches, falls back to default_icon/active_icon.
    #[serde(default)]
    pub icon_rules: Vec<IconRule>,

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

    /// Icon font size in points (None = use font_size)
    /// Nerd Font icons often need to be larger than text to appear balanced
    #[serde(default)]
    pub icon_font_size: Option<f32>,

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

/// Default macros for common application categories
fn default_macros() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();

    // Web browsers
    m.insert("browser".to_string(), vec![
        "firefox".to_string(),
        "brave-browser".to_string(),
        "chromium".to_string(),
        "google-chrome".to_string(),
        "qutebrowser".to_string(),
        "Navigator".to_string(),  // Firefox class
        "Chromium-browser".to_string(),
    ]);

    // File managers
    m.insert("fm".to_string(), vec![
        "nemo".to_string(),
        "nautilus".to_string(),
        "thunar".to_string(),
        "dolphin".to_string(),
        "pcmanfm".to_string(),
        "spacefm".to_string(),
        "caja".to_string(),
    ]);

    // Terminals
    m.insert("terminal".to_string(), vec![
        "kitty".to_string(),
        "alacritty".to_string(),
        "gnome-terminal".to_string(),
        "xterm".to_string(),
        "konsole".to_string(),
        "terminator".to_string(),
        "tilix".to_string(),
        "st".to_string(),
    ]);

    // Claude (AI assistant)
    m.insert("claude".to_string(), vec![
        "claude".to_string(),
        "Claude".to_string(),
    ]);

    // Code editors
    m.insert("editor".to_string(), vec![
        "code".to_string(),
        "Code".to_string(),
        "vscodium".to_string(),
        "sublime_text".to_string(),
        "atom".to_string(),
    ]);

    // JetBrains IDEs
    m.insert("jetbrains".to_string(), vec![
        "jetbrains-idea".to_string(),
        "jetbrains-pycharm".to_string(),
        "jetbrains-webstorm".to_string(),
        "jetbrains-clion".to_string(),
        "jetbrains-goland".to_string(),
        "jetbrains-rustrover".to_string(),
        "jetbrains-rider".to_string(),
        "jetbrains-datagrip".to_string(),
    ]);

    m
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // Macros - predefined class patterns
            macros: default_macros(),

            // Legacy fields - kept for backward compatibility
            //
            // Icon namespace gotcha: Panel fonts typically only support Material Design Icons
            // (󰀀-󿿿 range, U+F0000+). Codicons (U+E7xx) and Devicons (U+E6xx) render as boxes.
            // Stick to md-* icons from Nerd Fonts for reliable display.
            //
            // 󰝥 = md-circle (filled = has windows), 󰝣 = md-circle_outline (empty)
            default_icon: "󰝥".to_string(),
            active_icon: Some("󰝥".to_string()),
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
            // 󰝣 = md-circle_outline - empty/no windows
            empty_icon: Some("󰝣".to_string()),

            // Icon rules (empty by default - user configures)
            icon_rules: Vec::new(),

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
            icon_font_size: None,
            font_weight: None,

            // Layout
            button_padding: 4,
            max_button_width: None,
            expand_buttons: false,
        }
    }
}

impl Config {
    /// Merge user macros with defaults
    ///
    /// User-defined macros override defaults with the same name.
    /// Default macros not overridden by user are preserved.
    fn merge_macros(&mut self) {
        let defaults = default_macros();
        let user_macros_count = self.macros.len();
        let defaults_count = defaults.len();

        tracing::debug!(
            user_macros = user_macros_count,
            default_macros = defaults_count,
            "merging macros"
        );

        let mut added_count = 0;
        for (name, classes) in defaults {
            // Only insert if user didn't define this macro
            if self.macros.entry(name.clone()).or_insert_with(|| {
                added_count += 1;
                classes.clone()
            }).len() > 0 {
                tracing::trace!(
                    macro_name = %name,
                    class_count = classes.len(),
                    was_user_defined = added_count == 0,
                    "macro in final set"
                );
            }
        }

        tracing::debug!(
            total_macros = self.macros.len(),
            added_defaults = added_count,
            "macros merged"
        );
    }

    /// Load config from TOML file (with JSON fallback for migration)
    ///
    /// Tries .toml first, then falls back to .json for backward compatibility.
    /// If JSON is loaded, it will be migrated to TOML on next save.
    /// Default macros are merged with user-defined ones.
    pub fn load(path: &PathBuf) -> Result<Self> {
        let start = std::time::Instant::now();

        tracing::debug!(
            path = %path.display(),
            "loading config"
        );

        // Try TOML first
        let toml_path = path.with_extension("toml");
        if toml_path.exists() {
            tracing::trace!(path = %toml_path.display(), "attempting TOML load");
            match std::fs::read_to_string(&toml_path) {
                Ok(content) => {
                    tracing::trace!(
                        path = %toml_path.display(),
                        size_bytes = content.len(),
                        "read TOML file content"
                    );
                    match toml::from_str::<Config>(&content) {
                        Ok(mut config) => {
                            config.merge_macros();
                            tracing::info!(
                                path = %toml_path.display(),
                                icon_rules = config.icon_rules.len(),
                                macros = config.macros.len(),
                                elapsed_us = start.elapsed().as_micros(),
                                "config loaded from TOML"
                            );
                            return Ok(config);
                        }
                        Err(e) => {
                            tracing::error!(
                                path = %toml_path.display(),
                                error = %e,
                                elapsed_us = start.elapsed().as_micros(),
                                "failed to parse TOML config"
                            );
                            return Err(e.into());
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        path = %toml_path.display(),
                        error = %e,
                        elapsed_us = start.elapsed().as_micros(),
                        "failed to read TOML config file"
                    );
                    return Err(e.into());
                }
            }
        }

        // Fallback to JSON for backward compatibility
        let json_path = path.with_extension("json");
        if json_path.exists() {
            tracing::trace!(path = %json_path.display(), "attempting JSON load (fallback)");
            match std::fs::read_to_string(&json_path) {
                Ok(content) => {
                    tracing::trace!(
                        path = %json_path.display(),
                        size_bytes = content.len(),
                        "read JSON file content"
                    );
                    match serde_json::from_str::<Config>(&content) {
                        Ok(mut config) => {
                            config.merge_macros();
                            tracing::info!(
                                json_path = %json_path.display(),
                                toml_path = %toml_path.display(),
                                icon_rules = config.icon_rules.len(),
                                macros = config.macros.len(),
                                elapsed_us = start.elapsed().as_micros(),
                                "config loaded from JSON (will migrate to TOML on save)"
                            );
                            return Ok(config);
                        }
                        Err(e) => {
                            tracing::warn!(
                                path = %json_path.display(),
                                error = %e,
                                elapsed_us = start.elapsed().as_micros(),
                                "failed to parse JSON config"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        path = %json_path.display(),
                        error = %e,
                        "failed to read JSON config file"
                    );
                }
            }
        }

        // Also try the exact path (for edge cases)
        if path.exists() {
            tracing::trace!(path = %path.display(), "attempting exact path load");
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    // Try TOML first, then JSON
                    if let Ok(mut config) = toml::from_str::<Config>(&content) {
                        config.merge_macros();
                        tracing::info!(
                            path = %path.display(),
                            icon_rules = config.icon_rules.len(),
                            macros = config.macros.len(),
                            elapsed_us = start.elapsed().as_micros(),
                            "config loaded from exact path (TOML)"
                        );
                        return Ok(config);
                    }
                    if let Ok(mut config) = serde_json::from_str::<Config>(&content) {
                        config.merge_macros();
                        tracing::info!(
                            path = %path.display(),
                            icon_rules = config.icon_rules.len(),
                            macros = config.macros.len(),
                            elapsed_us = start.elapsed().as_micros(),
                            "config loaded from exact path (JSON)"
                        );
                        return Ok(config);
                    }
                    tracing::warn!(
                        path = %path.display(),
                        elapsed_us = start.elapsed().as_micros(),
                        "could not parse exact path as TOML or JSON"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to read exact path"
                    );
                }
            }
        }

        tracing::info!(
            path = %path.display(),
            elapsed_us = start.elapsed().as_micros(),
            "no config file found, using defaults"
        );
        Ok(Config::default())
    }

    /// Save config to TOML file
    ///
    /// Always saves as TOML (migrating from JSON if needed).
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let start = std::time::Instant::now();
        let toml_path = path.with_extension("toml");

        tracing::debug!(
            path = %toml_path.display(),
            icon_rules = self.icon_rules.len(),
            macros = self.macros.len(),
            "saving config"
        );

        if let Some(parent) = toml_path.parent() {
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

        let content = match toml::to_string_pretty(self) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    elapsed_us = start.elapsed().as_micros(),
                    "failed to serialize config to TOML"
                );
                return Err(e.into());
            }
        };

        tracing::trace!(
            path = %toml_path.display(),
            size_bytes = content.len(),
            "writing config to file"
        );

        if let Err(e) = std::fs::write(&toml_path, &content) {
            tracing::error!(
                path = %toml_path.display(),
                error = %e,
                elapsed_us = start.elapsed().as_micros(),
                "failed to write config file"
            );
            return Err(e.into());
        }

        tracing::info!(
            path = %toml_path.display(),
            size_bytes = content.len(),
            icon_rules = self.icon_rules.len(),
            macros = self.macros.len(),
            elapsed_us = start.elapsed().as_micros(),
            "config saved to TOML"
        );

        Ok(())
    }

    /// Get the TOML config path from a base path
    pub fn toml_path(base: &PathBuf) -> PathBuf {
        base.with_extension("toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_rule_with_class_regex() {
        let toml = r#"
            [[icon_rules]]
            class_regex = "^(firefox|chromium)$"
            icon = "󰖟"
            match_mode = "all"
            name = "Web browsers"
        "#;

        #[derive(Deserialize)]
        struct RulesWrapper { icon_rules: Vec<IconRule> }
        let wrapper: RulesWrapper = toml::from_str(toml).expect("should parse");
        assert_eq!(wrapper.icon_rules.len(), 1);
        assert_eq!(wrapper.icon_rules[0].match_mode, IconMatchMode::All);
        assert!(wrapper.icon_rules[0].class_regex.is_some());
    }

    #[test]
    fn test_icon_rule_with_macro() {
        let toml = r#"
            [[icon_rules]]
            macro = "browser"
            icon = "󰖟"
            match_mode = "all"
        "#;

        #[derive(Deserialize)]
        struct RulesWrapper { icon_rules: Vec<IconRule> }
        let wrapper: RulesWrapper = toml::from_str(toml).expect("should parse");
        assert_eq!(wrapper.icon_rules.len(), 1);
        assert!(wrapper.icon_rules[0].r#macro.is_some());
        assert_eq!(wrapper.icon_rules[0].r#macro.as_ref().unwrap(), "browser");
    }

    #[test]
    fn test_macro_expansion() {
        let mut macros = HashMap::new();
        macros.insert("browser".to_string(), vec![
            "firefox".to_string(),
            "brave-browser".to_string(),
        ]);

        let rule = IconRule {
            r#macro: Some("browser".to_string()),
            class_regex: None,
            icon: "󰖟".to_string(),
            match_mode: IconMatchMode::All,
            name: None,
        };

        // Should match when all windows are browsers
        assert!(rule.matches(&["firefox".to_string()], &macros));
        assert!(rule.matches(&["brave-browser".to_string()], &macros));
        assert!(rule.matches(&["firefox".to_string(), "brave-browser".to_string()], &macros));

        // Should NOT match when non-browser present (match_mode = all)
        assert!(!rule.matches(&["firefox".to_string(), "kitty".to_string()], &macros));
    }

    #[test]
    fn test_macro_expansion_any_mode() {
        let mut macros = HashMap::new();
        macros.insert("browser".to_string(), vec!["firefox".to_string()]);

        let rule = IconRule {
            r#macro: Some("browser".to_string()),
            class_regex: None,
            icon: "󰖟".to_string(),
            match_mode: IconMatchMode::Any,
            name: None,
        };

        // Should match when ANY window is a browser
        assert!(rule.matches(&["firefox".to_string(), "kitty".to_string()], &macros));
        assert!(!rule.matches(&["kitty".to_string()], &macros));
    }

    #[test]
    fn test_config_toml_parsing() {
        let toml = r#"
            default_icon = "○"

            [macros]
            browser = ["firefox", "brave-browser"]

            [[icon_rules]]
            macro = "browser"
            icon = "󰖟"
            match_mode = "all"
        "#;

        let config: Config = toml::from_str(toml).expect("should parse");
        assert_eq!(config.icon_rules.len(), 1);
        assert!(config.macros.contains_key("browser"));
        assert_eq!(config.macros["browser"].len(), 2);
    }

    #[test]
    fn test_default_macros_exist() {
        let config = Config::default();
        assert!(config.macros.contains_key("browser"));
        assert!(config.macros.contains_key("fm"));
        assert!(config.macros.contains_key("terminal"));
        assert!(config.macros.contains_key("claude"));
        assert!(config.macros.contains_key("editor"));
        assert!(config.macros.contains_key("jetbrains"));
    }
}
