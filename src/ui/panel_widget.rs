//! Workspace panel widget
//!
//! Displays workspace buttons with configurable icons/labels.
//! Supports emoji, FontAwesome, Nerd Fonts, or any Unicode text.
//! Right-click on buttons opens context menu for customization.
//!
//! ## Performance Architecture
//!
//! Widgets are created ONCE and reused. Updates are stratified by cost:
//!
//! | Update Type | When | Cost |
//! |-------------|------|------|
//! | `rebuild()` | Workspace count changes | ~10ms (full widget creation) |
//! | `update_state()` | Active workspace / CSS class changes | ~0.1ms |
//! | `update_dots()` | Provider animation tick | ~0.01ms (just queue_draw) |
//!
//! This enables 60fps dot animations while keeping CPU usage minimal.

use gdk;
use glib::prelude::IsA;
use glib::Propagation;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::app::{AppEvent, AppState};
use crate::config::WindowCountDisplay;
use crate::providers::RenderState;
use super::context_menu::build_workspace_menu;

/// Persistent state for a single workspace button
///
/// Stores widget references for in-place updates without recreation.
struct ButtonState {
    /// The button widget itself
    button: gtk::Button,
    /// Icon label (if present)
    icon: Option<gtk::Label>,
    /// Text label (if present)
    label: Option<gtk::Label>,
    /// Window count badge (if present)
    badge: Option<gtk::Label>,
    /// DrawingArea for provider dots - always present, renders when state available
    drawing_area: gtk::DrawingArea,
    /// Shared render state for dot animation - mutate this, then queue_draw()
    render_state: Rc<RefCell<Option<RenderState>>>,
    /// Workspace number this button represents
    workspace_number: i32,
}

/// Main workspace widget
pub struct WorkspaceWidget {
    /// Outer event box (for scroll events)
    event_box: gtk::EventBox,
    /// Inner container
    container: gtk::Box,
    /// Persistent button state - reused across updates
    buttons: Rc<RefCell<Vec<ButtonState>>>,
    /// Event sender for click handling
    tx: glib::Sender<AppEvent>,
    /// CSS provider for dynamic styles
    css_provider: gtk::CssProvider,
    /// Last rebuild timestamp for diagnostics
    last_rebuild: RefCell<Instant>,
    /// Cached workspace count to detect when rebuild is needed
    cached_workspace_count: RefCell<usize>,
    /// Last animation frame time - shared across all buttons for decay calculation
    ///
    /// Ring animations decay locally in richspace (not in babel daemon) to enable
    /// smooth 60fps animation without IPC overhead. Babel sends raw intensity on
    /// ActivityPulse events, richspace decays it frame by frame.
    last_frame: Rc<RefCell<Instant>>,
}

impl WorkspaceWidget {
    /// Create a new workspace widget
    pub fn new(state: &AppState, tx: glib::Sender<AppEvent>) -> Self {
        let start = Instant::now();
        tracing::debug!(
            workspace_count = state.workspaces.len(),
            orientation = ?state.orientation,
            spacing = state.config.spacing,
            "WorkspaceWidget::new BEGIN"
        );

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
            last_rebuild: RefCell::new(Instant::now()),
            cached_workspace_count: RefCell::new(0),
            last_frame: Rc::new(RefCell::new(Instant::now())),
        };

        // Apply CSS
        widget.apply_default_css(state);

        // Initial build
        widget.rebuild(state);

        tracing::debug!(
            elapsed_us = start.elapsed().as_micros(),
            "WorkspaceWidget::new END"
        );
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

    // =========================================================================
    // RENDER METHODS - Stratified by cost
    // =========================================================================

    /// Full render - smart dispatch to appropriate update method
    ///
    /// Automatically chooses the cheapest update path:
    /// - Workspace count changed → rebuild()
    /// - Otherwise → update_state()
    pub fn render(&self, state: &AppState) {
        let start = Instant::now();
        let current_count = state.workspaces.len();
        let cached_count = *self.cached_workspace_count.borrow();

        if current_count != cached_count {
            // Workspace count changed - need full rebuild
            tracing::debug!(
                old_count = cached_count,
                new_count = current_count,
                "Workspace count changed, rebuilding"
            );
            self.rebuild(state);
            tracing::debug!(elapsed_us = start.elapsed().as_micros(), "render() via rebuild complete");
        } else {
            // Just update state (CSS classes, labels, dots)
            self.update_state(state);
            tracing::debug!(elapsed_us = start.elapsed().as_micros(), "render() via update_state complete");
        }
    }

    /// Full rebuild - destroys and recreates all widgets
    ///
    /// EXPENSIVE (~10ms) - Only call when workspace COUNT changes.
    /// For state updates, use update_state() instead.
    pub fn rebuild(&self, state: &AppState) {
        let start = Instant::now();
        *self.last_rebuild.borrow_mut() = start;
        *self.cached_workspace_count.borrow_mut() = state.workspaces.len();

        tracing::debug!(
            workspace_count = state.workspaces.len(),
            "rebuild BEGIN"
        );

        // Clear existing buttons
        for child in self.container.children() {
            self.container.remove(&child);
        }
        self.buttons.borrow_mut().clear();

        // Create buttons for each workspace
        for ws in state.workspaces.iter() {
            let render_state = state.providers.get_render_state(ws.number).cloned();
            let button_state = self.create_button_state(ws, state, render_state, self.last_frame.clone());
            self.container.pack_start(&button_state.button, false, false, 0);
            self.buttons.borrow_mut().push(button_state);
        }

        self.container.show_all();

        tracing::debug!(
            elapsed_us = start.elapsed().as_micros(),
            workspace_count = state.workspaces.len(),
            "rebuild END"
        );
    }

    /// Update state - refreshes CSS classes, icon/label text, and dots
    ///
    /// CHEAP (~0.1ms) - Use for active workspace changes, rule matches, urgency, etc.
    pub fn update_state(&self, state: &AppState) {
        let start = Instant::now();
        let buttons = self.buttons.borrow();

        for (i, bs) in buttons.iter().enumerate() {
            if let Some(ws) = state.workspaces.get(i) {
                // Update CSS classes
                let ctx = bs.button.style_context();

                // Active state
                if ws.is_active {
                    ctx.add_class("active");
                } else {
                    ctx.remove_class("active");
                }

                // Window count state
                if ws.window_count > 0 {
                    ctx.add_class("has-windows");
                    ctx.remove_class("empty");
                } else {
                    ctx.remove_class("has-windows");
                    ctx.add_class("empty");
                }

                // Urgency from state file
                if let Some(ws_state) = state.state.get(ws.number) {
                    if ws_state.urgent.unwrap_or(false) {
                        ctx.add_class("urgent");
                    } else {
                        ctx.remove_class("urgent");
                    }

                    // Custom CSS class
                    if let Some(ref css_class) = ws_state.css_class {
                        ctx.add_class(css_class);
                    }
                }

                // Update icon and label text (for rule changes, custom labels, etc.)
                let (icon_text, label_text) = self.get_workspace_display(ws, state);

                if let Some(ref icon_label) = bs.icon {
                    if let Some(text) = icon_text {
                        icon_label.set_markup(&text);
                    }
                }

                if let Some(ref label) = bs.label {
                    if let Some(text) = label_text {
                        label.set_markup(&text);
                    }
                    // Update label active state CSS
                    let label_ctx = label.style_context();
                    if ws.is_active {
                        label_ctx.add_class("active");
                    } else {
                        label_ctx.remove_class("active");
                    }
                }

                // Update tooltip
                let tooltip = self.get_workspace_tooltip(ws, state);
                bs.button.set_tooltip_text(Some(&tooltip));

                // Update render state for dots and queue redraw
                if let Some(render_state) = state.providers.get_render_state(ws.number) {
                    // Update size if dot count changed
                    let new_dot_count = render_state.dots.len();
                    let width = if new_dot_count > 0 { (new_dot_count as i32 * 10).max(12) } else { 0 };
                    bs.drawing_area.set_size_request(width, state.size.max(16));

                    *bs.render_state.borrow_mut() = Some(render_state.clone());
                    bs.drawing_area.queue_draw();
                }
            }
        }

        tracing::debug!(
            elapsed_us = start.elapsed().as_micros(),
            button_count = buttons.len(),
            "update_state complete"
        );
    }

    /// Update dots only - just mutates render state and queues redraw
    ///
    /// VERY CHEAP (~0.01ms) - Use for 60fps dot animations.
    pub fn update_dots(&self, state: &AppState) {
        let start = Instant::now();
        let buttons = self.buttons.borrow();

        for (i, bs) in buttons.iter().enumerate() {
            if let Some(ws) = state.workspaces.get(i) {
                if let Some(render_state) = state.providers.get_render_state(ws.number) {
                    *bs.render_state.borrow_mut() = Some(render_state.clone());
                    bs.drawing_area.queue_draw();
                }
            }
        }

        tracing::trace!(
            elapsed_us = start.elapsed().as_micros(),
            "update_dots complete"
        );
    }

    /// Light update - just refresh CSS classes for active state
    ///
    /// DEPRECATED: Use update_state() instead.
    pub fn update_active(&self, state: &AppState) {
        self.update_state(state);
    }

    /// Queue redraw on all widgets (for animation updates)
    pub fn queue_redraw(&self) {
        self.container.queue_draw();
    }

    /// Refresh CSS styles from config
    pub fn refresh_css(&self, state: &AppState) {
        let start = Instant::now();
        self.apply_default_css(state);
        tracing::debug!(
            elapsed_us = start.elapsed().as_micros(),
            "CSS refreshed"
        );
    }

    // =========================================================================
    // WIDGET CREATION - Called only during rebuild()
    // =========================================================================

    /// Create persistent button state for a workspace
    fn create_button_state(
        &self,
        ws: &crate::wnck::WorkspaceInfo,
        state: &AppState,
        render_state: Option<RenderState>,
        last_frame: Rc<RefCell<Instant>>,
    ) -> ButtonState {
        let button = gtk::Button::new();
        self.add_css_to_widget(&button);
        button.style_context().add_class("richspace-button");

        // CSS classes for initial state
        if ws.is_active {
            button.style_context().add_class("active");
        }
        if ws.window_count > 0 {
            button.style_context().add_class("has-windows");
        } else {
            button.style_context().add_class("empty");
        }

        if let Some(ws_state) = state.state.get(ws.number) {
            if ws_state.urgent.unwrap_or(false) {
                button.style_context().add_class("urgent");
            }
            if let Some(ref css_class) = ws_state.css_class {
                button.style_context().add_class(css_class);
            }
        }

        // Get display components
        let (icon_text, label_text) = self.get_workspace_display(ws, state);

        // Build content
        let content_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        self.add_css_to_widget(&content_box);

        // Icon
        let icon = icon_text.map(|text| {
            let icon_label = gtk::Label::new(Some(&text));
            self.add_css_to_widget(&icon_label);
            icon_label.style_context().add_class("richspace-icon");
            icon_label.set_use_markup(true);
            icon_label
        });

        // Label
        let label = label_text.map(|text| {
            let label_widget = gtk::Label::new(Some(&text));
            self.add_css_to_widget(&label_widget);
            label_widget.style_context().add_class("richspace-label");
            if ws.is_active {
                label_widget.style_context().add_class("active");
            }
            label_widget.set_use_markup(true);
            label_widget
        });

        // Pack icon and label
        if state.config.icon_after_label {
            if let Some(ref l) = label {
                content_box.pack_start(l, false, false, 0);
            }
            if let Some(ref i) = icon {
                content_box.pack_start(i, false, false, 0);
            }
        } else {
            if let Some(ref i) = icon {
                content_box.pack_start(i, false, false, 0);
            }
            if let Some(ref l) = label {
                content_box.pack_start(l, false, false, 0);
            }
        }

        // Window count badge
        let badge = if ws.window_count > 0 {
            match state.config.window_count_display {
                WindowCountDisplay::Badge => {
                    let badge = gtk::Label::new(Some(&format!("{}", ws.window_count)));
                    self.add_css_to_widget(&badge);
                    badge.style_context().add_class("richspace-badge");
                    content_box.pack_end(&badge, false, false, 0);
                    Some(badge)
                }
                WindowCountDisplay::Inline => {
                    let count = gtk::Label::new(Some(&format!("({})", ws.window_count)));
                    self.add_css_to_widget(&count);
                    count.style_context().add_class("richspace-count");
                    content_box.pack_end(&count, false, false, 2);
                    Some(count)
                }
                _ => None,
            }
        } else {
            None
        };

        // Provider dots - shared state for animation
        // Always create DrawingArea so provider can connect later and dots will render
        let shared_render_state: Rc<RefCell<Option<RenderState>>> =
            Rc::new(RefCell::new(render_state));

        let drawing_area = {
            let da = gtk::DrawingArea::new();
            // Start with minimum size, will expand when dots arrive
            let dot_count = shared_render_state.borrow().as_ref().map(|r| r.dots.len()).unwrap_or(0);
            let width = if dot_count > 0 { (dot_count as i32 * 10).max(12) } else { 0 };
            let height = state.size.max(16);
            da.set_size_request(width, height);

            // Connect draw with shared state reference
            // Ring decay happens here - babel sends raw intensity, we decay locally for smooth 60fps
            let render_ref = shared_render_state.clone();
            let frame_ref = last_frame.clone();
            da.connect_draw(move |widget, ctx| {
                let mut state_opt = render_ref.borrow_mut();
                if let Some(ref mut render_state) = *state_opt {
                    // Calculate time since last frame for decay
                    let now = Instant::now();
                    let dt_secs = {
                        let mut last = frame_ref.borrow_mut();
                        let dt = now.duration_since(*last).as_secs_f32();
                        *last = now;
                        dt
                    };

                    let width = widget.allocated_width() as f64;
                    let height = widget.allocated_height() as f64;

                    let dot_radius = 3.0;
                    let spacing = 10.0;
                    let total_width = render_state.dots.len() as f64 * spacing;
                    let start_x = (width - total_width) / 2.0 + spacing / 2.0;
                    let center_y = height / 2.0;

                    let mut any_animating = false;
                    for (i, dot) in render_state.dots.iter_mut().enumerate() {
                        // Apply decay to ring intensity
                        let mut intensity = dot.ring_intensity as f32;
                        claude_babel::render::decay_ring(&mut intensity, dt_secs);
                        dot.ring_intensity = intensity as f64;

                        if intensity > 0.0 {
                            any_animating = true;
                        }

                        let x = start_x + i as f64 * spacing;
                        let style = spaceship_std::visual::DotStyle {
                            color: spaceship_std::visual::Rgb::new(dot.r, dot.g, dot.b),
                            ring_intensity: dot.ring_intensity,
                            ..Default::default()
                        };
                        claude_babel::render::render_dot(ctx, x, center_y, dot_radius, &style);
                    }

                    // Schedule next frame if still animating
                    if any_animating {
                        let widget_clone = widget.clone();
                        glib::idle_add_local_once(move || {
                            widget_clone.queue_draw();
                        });
                    }
                }
                Propagation::Proceed
            });

            content_box.pack_end(&da, false, false, 2);
            da
        };

        button.add(&content_box);

        // Tooltip
        let tooltip = self.get_workspace_tooltip(ws, state);
        button.set_tooltip_text(Some(&tooltip));

        // Click handlers
        let tx = self.tx.clone();
        let ws_num = ws.number;
        button.connect_clicked(move |_| {
            tx.send(AppEvent::WorkspaceClicked(ws_num)).ok();
        });

        let tx_menu = self.tx.clone();
        let ws_number = ws.number;
        let current_label = state.state.get(ws.number).and_then(|s| s.label.clone());
        let current_icon = state.state.get(ws.number).and_then(|s| s.icon.clone());

        button.connect_button_press_event(move |_, event| {
            if event.button() == 3 {
                let tx = tx_menu.clone();
                let ws_num = ws_number;
                let label = current_label.clone();
                let icon = current_icon.clone();

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

                menu.popup_at_pointer(Some(event));
                return Propagation::Stop;
            }
            Propagation::Proceed
        });

        ButtonState {
            button,
            icon,
            label,
            badge,
            drawing_area,
            render_state: shared_render_state,
            workspace_number: ws.number,
        }
    }

    // =========================================================================
    // CSS MANAGEMENT
    // =========================================================================

    fn apply_default_css(&self, state: &AppState) {
        let start = Instant::now();

        let mut css = String::from(r#"
        .richspace {
            padding: 0;
            margin: 0;
        }

        .richspace-button {
"#);

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
            transition: all 60ms ease;
        }

        .richspace-button:hover {
            background: rgba(255, 255, 255, 0.1);
            transition: none;
        }

        .richspace-button.active {
            background: alpha(@theme_selected_bg_color, 0.2);
            transition: none;
        }

        .richspace-button.active:hover {
            background: alpha(@theme_selected_bg_color, 0.3);
            transition: none;
        }

        .richspace-button.urgent {
            background: alpha(#e74c3c, 0.3);
            transition: none;
        }

        .richspace-icon {
            color: alpha(@theme_fg_color, 0.65);
"#);

        Self::append_icon_typography_css(&mut css, state);
        css.push_str("        }\n\n");

        css.push_str("        .richspace-label {\n            color: alpha(@theme_fg_color, 0.65);\n");
        Self::append_typography_css(&mut css, state);
        css.push_str("        }\n\n");

        css.push_str(r#"        .richspace-button.active .richspace-icon,
        .richspace-button.active .richspace-label {
            color: @theme_fg_color;
        }

        .richspace-button.empty .richspace-icon {
            color: alpha(@theme_fg_color, 0.375);
        }
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

        .richspace-button.claude-idle {
            border-bottom: 2px solid alpha(@theme_fg_color, 0.2);
        }

        .richspace-button.claude-busy {
            background: alpha(#cca133, 0.15);
            border-bottom: 2px solid #cca133;
        }

        .richspace-button.claude-busy-all {
            background: alpha(#cca133, 0.25);
            border-bottom: 3px solid #cca133;
        }

        .richspace-button.claude-await-low {
            background: alpha(#d27998, 0.15);
            border-bottom: 2px solid #d27998;
        }

        .richspace-button.claude-await-mid {
            background: alpha(#d27998, 0.25);
            border-bottom: 3px solid #d27998;
        }

        .richspace-button.claude-await-hot {
            background: alpha(#ad1f51, 0.3);
            border-bottom: 3px solid #ad1f51;
        }

        .richspace-badge.claude-count {
            background: alpha(@theme_fg_color, 0.3);
            font-size: 7pt;
            padding: 1px 4px;
            border-radius: 8px;
            margin-left: 4px;
        }
"#);

        if let Some(ref custom) = state.config.custom_css {
            css.push_str("\n        /* Custom CSS */\n");
            css.push_str(custom);
            css.push('\n');
        }

        if let Err(e) = self.css_provider.load_from_data(css.as_bytes()) {
            tracing::warn!(error = %e, "CSS parsing failed");
            return;
        }

        self.container.style_context().add_provider(
            &self.css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );

        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &self.css_provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }

        tracing::trace!(
            elapsed_us = start.elapsed().as_micros(),
            "apply_default_css complete"
        );
    }

    fn add_css_to_widget(&self, widget: &impl IsA<gtk::Widget>) {
        widget.style_context().add_provider(
            &self.css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
    }

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

    fn append_icon_typography_css(css: &mut String, state: &AppState) {
        if let Some(ref family) = state.config.font_family {
            css.push_str(&format!("            font-family: \"{}\";\n", family));
        }
        let icon_size = state.config.icon_font_size.or(state.config.font_size);
        if let Some(size) = icon_size {
            css.push_str(&format!("            font-size: {}pt;\n", size));
        }
        if let Some(ref weight) = state.config.font_weight {
            css.push_str(&format!("            font-weight: {};\n", weight));
        }
    }

    // =========================================================================
    // DISPLAY HELPERS
    // =========================================================================

    fn get_workspace_icon(&self, ws: &crate::wnck::WorkspaceInfo, state: &AppState) -> Option<String> {
        if let Some(ws_state) = state.state.get(ws.number) {
            if let Some(ref icon) = ws_state.icon {
                return Some(icon.clone());
            }
        }

        if ws.window_count == 0 {
            if state.config.hide_empty_icon {
                return None;
            }
            if let Some(ref empty_icon) = state.config.empty_icon {
                return Some(empty_icon.clone());
            }
        }

        if !ws.window_classes.is_empty() && !state.config.icon_rules.is_empty() {
            for rule in state.config.icon_rules.iter() {
                if rule.matches(&ws.window_classes, &state.config.macros) {
                    return Some(rule.icon.clone());
                }
            }
        }

        let icon = if ws.is_active {
            state.config.active_icon.clone().unwrap_or_else(|| state.config.default_icon.clone())
        } else {
            state.config.default_icon.clone()
        };

        Some(icon)
    }

    fn get_workspace_display(&self, ws: &crate::wnck::WorkspaceInfo, state: &AppState) -> (Option<String>, Option<String>) {
        use crate::config::{DisplayMode, LabelSource};

        let icon = self.get_workspace_icon(ws, state);

        let custom_label = state.state.get(ws.number).and_then(|s| s.label.clone());

        let label = if let Some(label) = custom_label {
            Some(label)
        } else {
            match state.config.label_source {
                LabelSource::Number => Some(format!("{}", ws.number + 1)),
                LabelSource::WmName => {
                    if ws.name.is_empty() {
                        Some(format!("{}", ws.number + 1))
                    } else {
                        Some(ws.name.clone())
                    }
                }
                LabelSource::Custom => Some(format!("{}", ws.number + 1)),
            }
        };

        let label = if state.config.active_only_label && !ws.is_active {
            None
        } else {
            label
        };

        match state.config.display_mode {
            DisplayMode::IconOnly => (icon, None),
            DisplayMode::LabelOnly => (None, label),
            DisplayMode::IconAndLabel => (icon, label),
        }
    }

    fn get_workspace_tooltip(&self, ws: &crate::wnck::WorkspaceInfo, state: &AppState) -> String {
        let urgent = state.state.get(ws.number)
            .and_then(|s| s.urgent)
            .unwrap_or(false);

        let mut tooltip = String::new();

        if urgent {
            tooltip.push_str("⚠️ ");
        }

        if let Some(ws_state) = state.state.get(ws.number) {
            if let Some(ref custom_tooltip) = ws_state.tooltip {
                tooltip.push_str(custom_tooltip);
                return tooltip;
            }
        }

        if state.config.show_name_tooltips && !ws.name.is_empty() {
            tooltip.push_str(&ws.name);
            if state.config.show_window_count {
                tooltip.push_str(&format!(" ({} windows)", ws.window_count));
            }
            return tooltip;
        }

        tooltip.push_str(&format!("Workspace {}", ws.number + 1));
        if state.config.show_window_count {
            tooltip.push_str(&format!(" ({} windows)", ws.window_count));
        }
        tooltip
    }
}
