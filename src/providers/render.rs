//! Provider render state types
//!
//! Defines the data structures providers send for custom workspace rendering.
//! RenderState is a declarative description of what to draw, not imperative
//! drawing commands - this keeps the protocol simple and efficient.
//!
//! ## Coordinate System
//!
//! All coordinates are normalized (0.0-1.0) relative to the workspace button area.
//! This allows providers to be resolution-independent.
//!
//! - `x`: 0.0 = left edge, 1.0 = right edge
//! - `y`: 0.0 = top edge, 1.0 = bottom edge
//! - `radius`: fraction of button height (0.3 = 30% of height)

use serde::{Deserialize, Serialize};

/// Complete render state for a workspace
///
/// Sent by providers to describe what should be drawn.
/// Richspace's cairo renderer interprets this state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderState {
    /// Dots to render (one per Claude session, etc.)
    #[serde(default)]
    pub dots: Vec<RenderDot>,

    /// Whether this workspace should show urgency indicator
    #[serde(default)]
    pub urgent: bool,

    /// Optional tooltip text
    #[serde(default)]
    pub tooltip: Option<String>,

    /// Optional label to show (overrides default workspace number)
    #[serde(default)]
    pub label: Option<String>,

    /// Optional icon (nerd font glyph or emoji)
    #[serde(default)]
    pub icon: Option<String>,

    /// Whether there's animation activity (triggers redraw loop)
    #[serde(default)]
    pub animating: bool,
}

/// A single dot to render
///
/// Represents one "entity" (Claude session, browser tab, etc.) as a colored dot
/// with optional ring glow animation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderDot {
    /// X position (0.0-1.0, left to right)
    pub x: f64,

    /// Y position (0.0-1.0, top to bottom)
    pub y: f64,

    /// Red component (0.0-1.0)
    pub r: f64,

    /// Green component (0.0-1.0)
    pub g: f64,

    /// Blue component (0.0-1.0)
    pub b: f64,

    /// Ring glow intensity (0.0-1.0)
    /// Animated glow effect around the dot during activity (token output)
    #[serde(default, alias = "pulse")]
    pub ring_intensity: f64,

    /// Optional hex color for a durable semantic ring.
    ///
    /// When absent, ring_intensity is a pulse and renderers may decay it.
    /// When present, babel is carrying domain state such as unread completion,
    /// so the ring should persist until the provider sends a later state
    /// without ring_color.
    #[serde(default)]
    pub ring_color: Option<String>,

    /// Dot radius as fraction of button height
    #[serde(default = "default_radius")]
    pub radius: f64,
}

fn default_radius() -> f64 {
    0.25
}

impl RenderDot {
    /// Create a new dot with basic properties
    pub fn new(x: f64, y: f64, r: f64, g: f64, b: f64) -> Self {
        Self {
            x,
            y,
            r,
            g,
            b,
            ring_intensity: 0.0,
            ring_color: None,
            radius: default_radius(),
        }
    }

    /// Create a dot with specific color (CSS hex format)
    pub fn with_hex(x: f64, y: f64, hex: &str) -> Self {
        let (r, g, b) = parse_hex_color(hex);
        Self::new(x, y, r, g, b)
    }
}

/// Parse hex color string to RGB (0.0-1.0)
fn parse_hex_color(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return (0.5, 0.5, 0.5); // Default gray
    }

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(128) as f64 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(128) as f64 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(128) as f64 / 255.0;

    (r, g, b)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Cairo Rendering
// ═══════════════════════════════════════════════════════════════════════════════

impl RenderState {
    /// Draw this render state to a cairo context
    ///
    /// Called by WorkspaceWidget when rendering provider-claimed workspaces.
    /// The context should already be translated to the button's area.
    pub fn draw(&self, ctx: &cairo::Context, width: f64, height: f64) {
        use std::f64::consts::TAU;

        for dot in &self.dots {
            // Convert normalized coordinates to pixels
            let x = dot.x * width;
            let y = dot.y * height;
            let radius = dot.radius * height;

            // Draw ring glow (if any) - animated effect during activity
            if dot.ring_intensity > 0.01 {
                let ring_radius = radius * (1.0 + dot.ring_intensity * 0.5);
                let ring_alpha = dot.ring_intensity * 0.4;

                let (ring_r, ring_g, ring_b) = dot
                    .ring_color
                    .as_deref()
                    .map(parse_hex_color)
                    .unwrap_or((dot.r, dot.g, dot.b));
                ctx.set_source_rgba(ring_r, ring_g, ring_b, ring_alpha);
                ctx.arc(x, y, ring_radius, 0.0, TAU);
                ctx.fill().ok();
            }

            // Draw main dot
            ctx.set_source_rgb(dot.r, dot.g, dot.b);
            ctx.arc(x, y, radius, 0.0, TAU);
            ctx.fill().ok();
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_state_serialization() {
        let state = RenderState {
            dots: vec![
                RenderDot::new(0.2, 0.5, 0.9, 0.7, 0.2),
                RenderDot::with_hex(0.8, 0.5, "#40c0f0"),
            ],
            urgent: true,
            tooltip: Some("2 sessions working".to_string()),
            label: None,
            icon: Some("󰚩".to_string()),
            animating: true,
        };

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("dots"));
        assert!(json.contains("urgent"));

        // Round-trip
        let parsed: RenderState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.dots.len(), 2);
        assert!(parsed.urgent);
    }

    #[test]
    fn test_hex_color_parsing() {
        let (r, g, b) = parse_hex_color("#ff0000");
        assert!((r - 1.0).abs() < 0.01);
        assert!(g.abs() < 0.01);
        assert!(b.abs() < 0.01);

        let (r, g, b) = parse_hex_color("00ff00");
        assert!(r.abs() < 0.01);
        assert!((g - 1.0).abs() < 0.01);
        assert!(b.abs() < 0.01);
    }
}
