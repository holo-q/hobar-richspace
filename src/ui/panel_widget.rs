//! Workspace panel widget
//!
//! Displays workspace buttons with configurable icons/labels.
//! Supports emoji, FontAwesome, Nerd Fonts, or any Unicode text.
//! Right-click on buttons opens context menu for customization.
//!
//! Provider-claimed workspaces use DrawingArea with custom cairo rendering
//! for advanced visuals (dots, pulses, etc.) driven by external processes.

use gdk;
use glib::prelude::IsA;
use glib::Propagation;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::app::{AppEvent, AppState};
use crate::config::WindowCountDisplay;
use crate::providers::RenderState;
use super::context_menu::build_workspace_menu;

/// Main workspace widget
pub struct WorkspaceWidget {
    /// Outer event box (for scroll events)
    event_box: gtk::EventBox,
    /// Inner container
    container: gtk::Box,
    /// Workspace buttons (recreated on workspace changes)
    buttons: Rc<RefCell<Vec<gtk::Button>>>,
    /// Event sender for click handling
    tx: glib::Sender<AppEvent>,
    /// CSS provider for dynamic styles
    css_provider: gtk::CssProvider,
}

impl WorkspaceWidget {
    /// Create a new workspace widget
    pub fn new(state: &AppState, tx: glib::Sender<AppEvent>) -> Self {
        let container = gtk::Box::new(state.orientation, state.config.spacing);
        container.style_context().add_class("richspace");

        // Wrap in EventBox for scroll events
        let event_box = gtk::EventBox::new();
        event_box.add(&container);
        event_box.add_events(gdk::EventMask::SCROLL_MASK);

        let css_provider = gtk::CssProvider::new();

        // Connect scroll event for workspace switching
        let scroll_enabled = state.config.scroll_enabled;
        let scroll_wrap = state.config.scroll_wrap;
        let tx_scroll = tx.clone();
        event_box.connect_scroll_event(move |_, event| {
            if !scroll_enabled {
                return Propagation::Proceed;
            }

            use gdk::ScrollDirection;
            let delta: i32 = match event.direction() {
                ScrollDirection::Up | ScrollDirection::Left => -1,
                ScrollDirection::Down | ScrollDirection::Right => 1,
                _ => return Propagation::Proceed,
            };

            tx_scroll.send(AppEvent::ScrollWorkspace { delta, wrap: scroll_wrap }).ok();
            Propagation::Stop
        });

        let widget = WorkspaceWidget {
            event_box,
            container: container.clone(),
            buttons: Rc::new(RefCell::new(Vec::new())),
            tx,
            css_provider,
        };

        // Apply default CSS with state for dynamic typography
        widget.apply_default_css(state);

        // Initial render
        widget.render(state);

        widget
    }

    /// Get the widget to add to the panel (EventBox wrapper)
    pub fn widget(&self) -> &gtk::EventBox {
        &self.event_box
    }

    /// Set orientation
    pub fn set_orientation(&self, orientation: gtk::Orientation) {
        self.container.set_orientation(orientation);
    }

    /// Apply default CSS styles (dynamically generated from config)
    ///
    /// Generates CSS based on:
    /// - button_padding: dynamic padding from config
    /// - font_family, font_size, font_weight: typography settings
    /// - custom_css: appended last to allow user overrides
    ///
    /// # GTK CSS Gotchas (learned the hard way)
    ///
    /// 1. **No `@keyframes`**: GTK CSS does NOT support CSS animations via `@keyframes`.
    ///    If included, the ENTIRE CSS fails to parse silently. Use static styles or
    ///    programmatic animation via GLib timeouts instead.
    ///
    /// 2. **Screen-wide CSS unreliable in panels**: `add_provider_for_screen()` alone doesn't
    ///    work for XFCE panel plugins. Must ALSO call `add_provider()` on each widget's
    ///    StyleContext directly.
    ///
    /// 3. **`opacity` doesn't work on Labels**: GTK CSS `opacity` property is unreliable on
    ///    gtk::Label widgets, especially inside buttons. Use `color: alpha(@theme_fg_color, 0.5)`
    ///    instead to achieve dimming.
    ///
    /// 4. **`currentColor` and `inherit` broken**: These CSS values don't work reliably in
    ///    GTK CSS for panel plugins. Always use explicit color values like `@theme_fg_color`.
    ///
    /// 5. **Theme colors**: Use `@theme_fg_color`, `@theme_bg_color`, `@theme_selected_bg_color`
    ///    etc. to inherit from the user's GTK theme.
    fn apply_default_css(&self, state: &AppState) {
        // Build dynamic CSS based on config
        let mut css = String::from(r#"
        .richspace {
            padding: 0;
            margin: 0;
        }

        .richspace-button {
"#);

        // Dynamic button padding from config
        css.push_str(&format!("            padding: {}px {}px;\n",
            state.config.button_padding,
            state.config.button_padding + 4));

        css.push_str(r#"            margin: 0;
            border-radius: 4px;
            min-width: 0;
            min-height: 0;
            background: transparent;
            border: none;
            box-shadow: none;
            /* Fade OUT only: transition on base state controls return animation */
            transition: all 60ms ease;
        }

        .richspace-button:hover {
            background: rgba(255, 255, 255, 0.1);
            /* No fade IN: instant snap to hover state */
            transition: none;
        }

        /* Active workspace: subtle highlight */
        .richspace-button.active {
            background: alpha(@theme_selected_bg_color, 0.2);
            transition: none;  /* Instant snap when becoming active */
        }

        .richspace-button.active:hover {
            background: alpha(@theme_selected_bg_color, 0.3);
            transition: none;
        }

        /* Urgency - solid highlight (GTK CSS doesn't support @keyframes) */
        .richspace-button.urgent {
            background: alpha(#e74c3c, 0.3);
            transition: none;
        }

        /* Icon styling - slightly dimmed by default */
        .richspace-icon {
            color: alpha(@theme_fg_color, 0.65);
"#);

        // Typography for icon (uses icon_font_size if set)
        Self::append_icon_typography_css(&mut css, state);
        css.push_str("        }\n\n");

        // Label styling - slightly dimmed by default
        css.push_str("        .richspace-label {\n            color: alpha(@theme_fg_color, 0.65);\n");
        // Typography for label
        Self::append_typography_css(&mut css, state);
        css.push_str("        }\n\n");

        // Active state - full brightness
        css.push_str(r#"        .richspace-button.active .richspace-icon,
        .richspace-button.active .richspace-label {
            color: @theme_fg_color;
        }

        /* Empty workspaces - more dimmed */
        .richspace-button.empty .richspace-icon,
        .richspace-button.empty .richspace-label {
            color: alpha(@theme_fg_color, 0.4);
        }

        .richspace-label.active {
            font-weight: bold;
        }

        .richspace-badge {
            font-size: 8pt;
            font-weight: bold;
            min-width: 14px;
            min-height: 14px;
            border-radius: 50%;
            background: @theme_selected_bg_color;
            color: @theme_selected_fg_color;
            padding: 0 3px;
        }

        .richspace-count {
            font-size: 9pt;
            opacity: 0.7;
        }

        /* ═══════════════════════════════════════════════════════════════════
         * Claude-Babel Integration CSS Classes
         * ═══════════════════════════════════════════════════════════════════
         * Applied by richspace-babel orchestrator daemon based on Claude
         * session activity state. Uses palette colors from palette.toml.
         */

        /* Claude present but idle - subtle indicator */
        .richspace-button.claude-idle {
            border-bottom: 2px solid alpha(@theme_fg_color, 0.2);
        }

        /* At least one Claude working - gold accent */
        .richspace-button.claude-busy {
            background: alpha(#cca133, 0.15);
            border-bottom: 2px solid #cca133;
        }

        /* All Claudes busy - stronger gold accent */
        .richspace-button.claude-busy-all {
            background: alpha(#cca133, 0.25);
            border-bottom: 3px solid #cca133;
        }

        /* Awaiting input - gradient levels based on elapsed time */
        /* Low (0-30s): subtle rose */
        .richspace-button.claude-await-low {
            background: alpha(#d27998, 0.15);
            border-bottom: 2px solid #d27998;
        }

        /* Medium (30-60s): growing rose */
        .richspace-button.claude-await-mid {
            background: alpha(#d27998, 0.25);
            border-bottom: 3px solid #d27998;
        }

        /* Hot (60s+): urgent red - combined with .urgent for pulsing */
        .richspace-button.claude-await-hot {
            background: alpha(#ad1f51, 0.3);
            border-bottom: 3px solid #ad1f51;
        }

        /* Badge for Claude window count per workspace */
        .richspace-badge.claude-count {
            background: alpha(@theme_fg_color, 0.3);
            font-size: 7pt;
            padding: 1px 4px;
            border-radius: 8px;
            margin-left: 4px;
        }
"#);

        // Custom CSS from config (appended last to allow overrides)
        if let Some(ref custom) = state.config.custom_css {
            css.push_str("\n        /* Custom CSS */\n");
            css.push_str(custom);
            css.push('\n');
        }

        // Load CSS - log errors prominently since GTK CSS fails silently on syntax errors
        // (e.g., @keyframes is not supported and will cause entire CSS to fail)
        if let Err(e) = self.css_provider.load_from_data(css.as_bytes()) {
            tracing::error!("CSS parsing failed: {}", e);
            tracing::error!("CSS content ({} bytes):\n{}", css.len(), css);
            return;
        }

        // Add provider directly to container's style context
        // Screen-wide CSS can be unreliable in panel plugins
        self.container.style_context().add_provider(
            &self.css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );

        // Also add to screen for child widgets
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &self.css_provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
    }

    /// Add CSS provider to a widget's style context
    fn add_css_to_widget(&self, widget: &impl IsA<gtk::Widget>) {
        widget.style_context().add_provider(
            &self.css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }

    /// Append typography CSS properties based on config
    ///
    /// Applies font_family, font_size, and font_weight if configured.
    /// If not set, GTK uses system defaults.
    fn append_typography_css(css: &mut String, state: &AppState) {
        if let Some(ref family) = state.config.font_family {
            css.push_str(&format!("            font-family: \"{}\";\n", family));
        }

        if let Some(size) = state.config.font_size {
            css.push_str(&format!("            font-size: {}pt;\n", size));
        }

        if let Some(ref weight) = state.config.font_weight {
            css.push_str(&format!("            font-weight: {};\n", weight));
        }
    }

    /// Append icon-specific typography CSS
    ///
    /// Uses icon_font_size if set, otherwise falls back to font_size.
    /// Nerd Font icons often need larger sizes to appear balanced with text.
    fn append_icon_typography_css(css: &mut String, state: &AppState) {
        if let Some(ref family) = state.config.font_family {
            css.push_str(&format!("            font-family: \"{}\";\n", family));
        }

        // Icon font size: use icon_font_size if set, else font_size
        let icon_size = state.config.icon_font_size.or(state.config.font_size);
        if let Some(size) = icon_size {
            css.push_str(&format!("            font-size: {}pt;\n", size));
        }

        if let Some(ref weight) = state.config.font_weight {
            css.push_str(&format!("            font-weight: {};\n", weight));
        }
    }

    /// Full render - rebuilds all workspace buttons
    ///
    /// Use this when workspace list changes or provider connections change.
    /// For lighter updates (active workspace, animation), use update_active() or queue_redraw().
    pub fn render(&self, state: &AppState) {
        // Refresh CSS (supports live config reload for typography/padding)
        self.apply_default_css(state);

        // Clear existing buttons
        for child in self.container.children() {
            self.container.remove(&child);
        }
        self.buttons.borrow_mut().clear();

        // Create buttons for each workspace
        // Provider dots are drawn INSIDE the button (integrated, not beside)
        for ws in &state.workspaces {
            // Get provider render state if available
            let render_state = state.providers.get_render_state(ws.number).cloned();

            // Create workspace button with optional provider dots inside
            let button = self.create_workspace_button(ws, state, render_state.as_ref());
            self.container.pack_start(&button, false, false, 0);
            self.buttons.borrow_mut().push(button);
        }

        self.container.show_all();
    }

    /// Light update - just refresh CSS classes for active state
    ///
    /// Much faster than full render. Use for workspace switches.
    pub fn update_active(&self, state: &AppState) {
        let buttons = self.buttons.borrow();
        for (i, button) in buttons.iter().enumerate() {
            let is_active = state.workspaces.get(i)
                .map(|ws| ws.is_active)
                .unwrap_or(false);

            let ctx = button.style_context();
            if is_active {
                ctx.add_class("active");
            } else {
                ctx.remove_class("active");
            }
        }
    }

    /// Queue redraw on all widgets (for animation updates)
    ///
    /// Triggers repaint without rebuilding widgets.
    pub fn queue_redraw(&self) {
        self.container.queue_draw();
    }

    /// Create a button for a workspace
    ///
    /// Builds button content based on DisplayMode:
    /// - IconOnly: shows icon only
    /// - LabelOnly: shows label only
    /// - IconAndLabel: shows both (icon + label side by side)
    ///
    /// If render_state is provided (provider claims this workspace), provider dots
    /// are drawn inside the button after the label/icon.
    fn create_workspace_button(
        &self,
        ws: &crate::wnck::WorkspaceInfo,
        state: &AppState,
        render_state: Option<&RenderState>,
    ) -> gtk::Button {
        let button = gtk::Button::new();
        self.add_css_to_widget(&button);
        button.style_context().add_class("richspace-button");

        // CSS classes for state
        if ws.is_active {
            button.style_context().add_class("active");
        }
        if ws.window_count > 0 {
            button.style_context().add_class("has-windows");
        } else {
            button.style_context().add_class("empty");
        }

        // Check for urgency and custom CSS class from state file
        if let Some(ws_state) = state.state.get(ws.number) {
            // Urgency indicator - triggers pulsing animation when workspace needs attention
            if ws_state.urgent.unwrap_or(false) {
                button.style_context().add_class("urgent");
            }

            // Custom CSS class from state file - allows external tools to apply custom styling
            if let Some(ref css_class) = ws_state.css_class {
                button.style_context().add_class(css_class);
            }
        }

        // Get display components
        let (icon, label_text) = self.get_workspace_display(ws, state);

        // Build button content
        let content_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        self.add_css_to_widget(&content_box);

        if let Some(icon_str) = icon {
            let icon_label = gtk::Label::new(Some(&icon_str));
            self.add_css_to_widget(&icon_label);
            icon_label.style_context().add_class("richspace-icon");
            icon_label.set_use_markup(true);
            content_box.pack_start(&icon_label, false, false, 0);
        }

        if let Some(label_str) = label_text {
            let label = gtk::Label::new(Some(&label_str));
            self.add_css_to_widget(&label);
            label.style_context().add_class("richspace-label");
            if ws.is_active {
                label.style_context().add_class("active");
            }
            label.set_use_markup(true);
            content_box.pack_start(&label, false, false, 0);
        }

        // Window count display (Badge or Inline mode)
        if ws.window_count > 0 {
            match state.config.window_count_display {
                WindowCountDisplay::Badge => {
                    let badge = gtk::Label::new(Some(&format!("{}", ws.window_count)));
                    self.add_css_to_widget(&badge);
                    badge.style_context().add_class("richspace-badge");
                    content_box.pack_end(&badge, false, false, 0);
                }
                WindowCountDisplay::Inline => {
                    let count = gtk::Label::new(Some(&format!("({})", ws.window_count)));
                    self.add_css_to_widget(&count);
                    count.style_context().add_class("richspace-count");
                    content_box.pack_end(&count, false, false, 2);
                }
                _ => {} // Hidden or Tooltip - no visual in button
            }
        }

        // Provider dots (if provider claims this workspace)
        // Drawn as small circles matching the window indicator style
        if let Some(render_state) = render_state {
            let dot_count = render_state.dots.len();
            if dot_count > 0 {
                let drawing_area = gtk::DrawingArea::new();
                // Each dot ~8px wide + 2px spacing
                let width = (dot_count as i32 * 10).max(12);
                let height = state.size.max(16);
                drawing_area.set_size_request(width, height);

                let render_state = render_state.clone();
                drawing_area.connect_draw(move |widget, ctx| {
                    let width = widget.allocated_width() as f64;
                    let height = widget.allocated_height() as f64;

                    // Draw dots - small filled circles like window indicators
                    let dot_radius = 3.0;
                    let spacing = 10.0;
                    let total_width = render_state.dots.len() as f64 * spacing;
                    let start_x = (width - total_width) / 2.0 + spacing / 2.0;
                    let center_y = height / 2.0;

                    for (i, dot) in render_state.dots.iter().enumerate() {
                        let x = start_x + i as f64 * spacing;

                        // Pulse glow effect (outer ring)
                        if dot.pulse > 0.01 {
                            let glow_radius = dot_radius + dot.pulse as f64 * 3.0;
                            ctx.set_source_rgba(dot.r, dot.g, dot.b, dot.pulse as f64 * 0.4);
                            ctx.arc(x, center_y, glow_radius, 0.0, std::f64::consts::TAU);
                            ctx.fill().ok();
                        }

                        // Main dot (filled circle)
                        ctx.set_source_rgb(dot.r, dot.g, dot.b);
                        ctx.arc(x, center_y, dot_radius, 0.0, std::f64::consts::TAU);
                        ctx.fill().ok();
                    }

                    glib::Propagation::Proceed
                });

                content_box.pack_end(&drawing_area, false, false, 2);
            }
        }

        button.add(&content_box);

        // Tooltip
        let tooltip = self.get_workspace_tooltip(ws, state);
        button.set_tooltip_text(Some(&tooltip));

        // Left-click handler - switch to workspace
        let tx = self.tx.clone();
        let ws_num = ws.number;
        button.connect_clicked(move |_| {
            tx.send(AppEvent::WorkspaceClicked(ws_num)).ok();
        });

        // Right-click handler - show context menu for customization
        let tx_menu = self.tx.clone();
        let ws_number = ws.number;

        // Get current values for the menu (clone them for the closure)
        let current_label = state.state.get(ws.number)
            .and_then(|s| s.label.clone());
        let current_icon = state.state.get(ws.number)
            .and_then(|s| s.icon.clone());

        button.connect_button_press_event(move |_, event| {
            if event.button() == 3 {  // Right-click (button 3)
                let tx = tx_menu.clone();
                let ws_num = ws_number;
                let label = current_label.clone();
                let icon = current_icon.clone();

                // Build context menu with callbacks for label/icon/clear
                let menu = build_workspace_menu(
                    ws_num,
                    label,
                    icon,
                    {
                        let tx = tx.clone();
                        move |new_label| {
                            tx.send(AppEvent::SetWorkspaceLabel {
                                workspace: ws_num,
                                label: new_label
                            }).ok();
                        }
                    },
                    {
                        let tx = tx.clone();
                        move |new_icon| {
                            tx.send(AppEvent::SetWorkspaceIcon {
                                workspace: ws_num,
                                icon: new_icon
                            }).ok();
                        }
                    },
                    {
                        let tx = tx.clone();
                        move || {
                            tx.send(AppEvent::ClearWorkspaceCustomizations {
                                workspace: ws_num
                            }).ok();
                        }
                    },
                );

                // Show menu at pointer position
                menu.popup_at_pointer(Some(event));
                return Propagation::Stop;  // Consume the event
            }
            Propagation::Proceed
        });

        button
    }

    /// Get the display icon for a workspace
    ///
    /// Returns workspace icon based on (in priority order):
    /// 1. Ephemeral state (if set via richspace-ctl set-icon)
    /// 2. Empty workspace icon (if no windows and empty_icon is set)
    /// 3. Icon rules (first matching rule wins)
    /// 4. Active vs default icon from config
    fn get_workspace_icon(&self, ws: &crate::wnck::WorkspaceInfo, state: &AppState) -> Option<String> {
        // Check ephemeral state first (user explicitly set icon)
        if let Some(ws_state) = state.state.get(ws.number) {
            if let Some(ref icon) = ws_state.icon {
                return Some(icon.clone());
            }
        }

        // Empty workspace icon
        if ws.window_count == 0 {
            if let Some(ref empty_icon) = state.config.empty_icon {
                return Some(empty_icon.clone());
            }
        }

        // Evaluate icon rules (first match wins)
        // Rules can reference macros (predefined class patterns) or raw regex
        if !ws.window_classes.is_empty() && !state.config.icon_rules.is_empty() {
            tracing::debug!(
                workspace = ws.number,
                classes = ?ws.window_classes,
                rule_count = state.config.icon_rules.len(),
                "Evaluating icon rules"
            );
            for rule in &state.config.icon_rules {
                if rule.matches(&ws.window_classes, &state.config.macros) {
                    tracing::info!(
                        workspace = ws.number,
                        rule = ?rule.name,
                        icon = %rule.icon,
                        "Icon rule matched"
                    );
                    return Some(rule.icon.clone());
                }
            }
            tracing::debug!(workspace = ws.number, "No icon rule matched");
        }

        // Active vs default icon
        if ws.is_active {
            Some(state.config.active_icon.clone().unwrap_or_else(|| state.config.default_icon.clone()))
        } else {
            Some(state.config.default_icon.clone())
        }
    }

    /// Get the display components for a workspace
    ///
    /// Returns (icon, label) - either can be None based on DisplayMode and LabelSource.
    /// State file labels always take priority (set via right-click menu).
    /// Falls back to label_source config when no custom label is set.
    fn get_workspace_display(&self, ws: &crate::wnck::WorkspaceInfo, state: &AppState) -> (Option<String>, Option<String>) {
        use crate::config::{DisplayMode, LabelSource};

        // Get icon
        let icon = self.get_workspace_icon(ws, state);

        // State file label takes priority (user explicitly set it via context menu)
        let custom_label = state.state.get(ws.number).and_then(|s| s.label.clone());

        let label = if let Some(label) = custom_label {
            Some(label)
        } else {
            // Fall back to label_source config
            match state.config.label_source {
                LabelSource::Number => Some(format!("{}", ws.number + 1)),
                LabelSource::WmName => {
                    if ws.name.is_empty() {
                        Some(format!("{}", ws.number + 1))  // Fallback
                    } else {
                        Some(ws.name.clone())
                    }
                }
                LabelSource::Custom => Some(format!("{}", ws.number + 1)),  // No custom set, fallback
            }
        };

        // Apply active_only_label: non-active workspaces get no label
        let label = if state.config.active_only_label && !ws.is_active {
            None
        } else {
            label
        };

        // Filter based on display mode
        match state.config.display_mode {
            DisplayMode::IconOnly => (icon, None),
            DisplayMode::LabelOnly => (None, label),
            DisplayMode::IconAndLabel => (icon, label),
        }
    }

    /// Get the tooltip for a workspace
    fn get_workspace_tooltip(&self, ws: &crate::wnck::WorkspaceInfo, state: &AppState) -> String {
        // Check for urgency state to prepend warning indicator
        let urgent = state.state.get(ws.number)
            .and_then(|s| s.urgent)
            .unwrap_or(false);

        let mut tooltip = String::new();

        // Prepend urgency indicator if workspace needs attention
        if urgent {
            tooltip.push_str("⚠️ ");
        }

        // Check for custom tooltip in ephemeral state
        if let Some(ws_state) = state.state.get(ws.number) {
            if let Some(ref custom_tooltip) = ws_state.tooltip {
                tooltip.push_str(custom_tooltip);
                return tooltip;
            }
        }

        // Use workspace name if enabled
        if state.config.show_name_tooltips && !ws.name.is_empty() {
            tooltip.push_str(&ws.name);
            if state.config.show_window_count {
                tooltip.push_str(&format!(" ({} windows)", ws.window_count));
            }
            return tooltip;
        }

        // Default tooltip
        tooltip.push_str(&format!("Workspace {}", ws.number + 1));
        if state.config.show_window_count {
            tooltip.push_str(&format!(" ({} windows)", ws.window_count));
        }
        tooltip
    }
}
