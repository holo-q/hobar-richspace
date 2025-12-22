//! Reproduction case for slow GTK3 widget rebuild performance
//!
//! This version tests MULTIPLE render strategies to isolate what's expensive:
//! 1. Full rebuild (destroy + recreate all widgets) - CURRENT APPROACH
//! 2. Reuse widgets (just update classes/text)
//! 3. Animation only (queue_draw on DrawingAreas)
//!
//! Run: cargo run --release

use glib::Propagation;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea, EventBox,
    Label,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

const NUM_WORKSPACES: usize = 12;

#[derive(Clone)]
struct WorkspaceInfo {
    number: i32,
    is_active: bool,
    window_count: i32,
}

#[derive(Clone)]
struct ProviderDot {
    r: f64,
    g: f64,
    b: f64,
    phase: f64, // Animation phase 0.0-1.0
}

/// Stored button state for reuse strategy
struct ButtonState {
    button: Button,
    icon: Label,
    label: Label,
    drawing_area: Option<DrawingArea>,
    // Animation state that changes each frame
    dots: Rc<RefCell<Vec<ProviderDot>>>,
}

struct WorkspaceWidget {
    event_box: EventBox,
    container: GtkBox,
    css_provider: CssProvider,
    // Persistent button state (for reuse strategy)
    buttons: Rc<RefCell<Vec<ButtonState>>>,
    timing_label: Label,
    render_count: Rc<RefCell<u32>>,
}

impl WorkspaceWidget {
    fn new(timing_label: Label) -> Self {
        let container = GtkBox::new(gtk::Orientation::Horizontal, 2);
        container.style_context().add_class("workspace-widget");

        let event_box = EventBox::new();
        event_box.add(&container);

        let css_provider = CssProvider::new();
        Self::load_css(&css_provider);

        container
            .style_context()
            .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);

        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &css_provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }

        Self {
            event_box,
            container,
            css_provider,
            buttons: Rc::new(RefCell::new(Vec::new())),
            timing_label,
            render_count: Rc::new(RefCell::new(0)),
        }
    }

    fn load_css(provider: &CssProvider) {
        let css = r#"
            .workspace-widget { padding: 0; margin: 0; background: #2d2d2d; }
            .richspace-button {
                padding: 4px 8px; margin: 0; border-radius: 4px;
                min-width: 0; min-height: 0;
                background: transparent; border: none;
            }
            .richspace-button.active { background: alpha(@theme_selected_bg_color, 0.2); }
            .richspace-icon { color: alpha(@theme_fg_color, 0.65); font-size: 11pt; }
            .richspace-label { color: alpha(@theme_fg_color, 0.65); font-size: 10pt; }
            .richspace-button.active .richspace-icon,
            .richspace-button.active .richspace-label { color: @theme_fg_color; }
        "#;
        provider
            .load_from_data(css.as_bytes())
            .expect("CSS parse error");
    }

    fn widget(&self) -> &EventBox {
        &self.event_box
    }

    fn add_css(&self, widget: &impl glib::prelude::IsA<gtk::Widget>) {
        widget
            .style_context()
            .add_provider(&self.css_provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
    }

    // =========================================================================
    // STRATEGY 1: Full rebuild (current approach - SLOW)
    // =========================================================================
    fn render_full_rebuild(&self, workspaces: &[WorkspaceInfo], dots: &[ProviderDot]) {
        let start = Instant::now();

        // Clear
        let t0 = Instant::now();
        for child in self.container.children() {
            self.container.remove(&child);
        }
        self.buttons.borrow_mut().clear();
        let clear_us = t0.elapsed().as_micros();

        // Create widgets
        let t1 = Instant::now();
        let mut widget_times: Vec<u128> = Vec::new();

        for ws in workspaces {
            let tw = Instant::now();

            let button = Button::new();
            self.add_css(&button);
            button.style_context().add_class("richspace-button");
            if ws.is_active {
                button.style_context().add_class("active");
            }

            let content = GtkBox::new(gtk::Orientation::Horizontal, 2);
            self.add_css(&content);

            let icon = Label::new(Some("●"));
            self.add_css(&icon);
            icon.style_context().add_class("richspace-icon");
            content.pack_start(&icon, false, false, 0);

            let label = Label::new(Some(&format!("{}", ws.number + 1)));
            self.add_css(&label);
            label.style_context().add_class("richspace-label");
            content.pack_start(&label, false, false, 0);

            // DrawingArea for dots
            if ws.window_count > 0 && !dots.is_empty() {
                let da = DrawingArea::new();
                da.set_size_request(24, 24);
                let dots_clone: Vec<_> = dots.to_vec();
                da.connect_draw(move |_, ctx| {
                    for (i, dot) in dots_clone.iter().enumerate() {
                        let x = 6.0 + i as f64 * 10.0;
                        let pulse = 0.8 + 0.2 * (dot.phase * std::f64::consts::TAU).sin();
                        ctx.arc(x, 12.0, 3.0 * pulse, 0.0, std::f64::consts::TAU);
                        ctx.set_source_rgb(dot.r, dot.g, dot.b);
                        ctx.fill().ok();
                    }
                    Propagation::Proceed
                });
                content.pack_end(&da, false, false, 2);
            }

            button.add(&content);
            self.container.pack_start(&button, false, false, 0);

            widget_times.push(tw.elapsed().as_micros());
        }
        let create_us = t1.elapsed().as_micros();

        // Show
        let t2 = Instant::now();
        self.container.show_all();
        let show_us = t2.elapsed().as_micros();

        let total_us = start.elapsed().as_micros();
        self.update_timing("FULL REBUILD", total_us, clear_us, create_us, show_us, &widget_times);
    }

    // =========================================================================
    // STRATEGY 2: Reuse widgets (only update what changed)
    // =========================================================================
    fn render_reuse(&self, workspaces: &[WorkspaceInfo], dots: &[ProviderDot]) {
        let start = Instant::now();
        let mut buttons = self.buttons.borrow_mut();

        // First time: create widgets
        if buttons.is_empty() {
            drop(buttons);
            self.create_persistent_buttons(workspaces, dots);
            let total_us = start.elapsed().as_micros();
            self.update_timing("REUSE (init)", total_us, 0, total_us, 0, &[]);
            return;
        }

        // Update existing widgets
        let t0 = Instant::now();
        for (i, ws) in workspaces.iter().enumerate() {
            if let Some(bs) = buttons.get_mut(i) {
                // Update active class
                let ctx = bs.button.style_context();
                if ws.is_active {
                    ctx.add_class("active");
                } else {
                    ctx.remove_class("active");
                }

                // Update dots animation state
                *bs.dots.borrow_mut() = dots.to_vec();

                // Queue redraw on drawing area only
                if let Some(ref da) = bs.drawing_area {
                    da.queue_draw();
                }
            }
        }
        let update_us = t0.elapsed().as_micros();

        let total_us = start.elapsed().as_micros();
        self.update_timing("REUSE", total_us, 0, update_us, 0, &[]);
    }

    fn create_persistent_buttons(&self, workspaces: &[WorkspaceInfo], dots: &[ProviderDot]) {
        let mut buttons = self.buttons.borrow_mut();

        for ws in workspaces {
            let button = Button::new();
            self.add_css(&button);
            button.style_context().add_class("richspace-button");
            if ws.is_active {
                button.style_context().add_class("active");
            }

            let content = GtkBox::new(gtk::Orientation::Horizontal, 2);
            self.add_css(&content);

            let icon = Label::new(Some("●"));
            self.add_css(&icon);
            icon.style_context().add_class("richspace-icon");
            content.pack_start(&icon, false, false, 0);

            let label = Label::new(Some(&format!("{}", ws.number + 1)));
            self.add_css(&label);
            label.style_context().add_class("richspace-label");
            content.pack_start(&label, false, false, 0);

            let dots_state: Rc<RefCell<Vec<ProviderDot>>> =
                Rc::new(RefCell::new(dots.to_vec()));

            let drawing_area = if ws.window_count > 0 {
                let da = DrawingArea::new();
                da.set_size_request(24, 24);
                let dots_ref = dots_state.clone();
                da.connect_draw(move |_, ctx| {
                    let dots = dots_ref.borrow();
                    for (i, dot) in dots.iter().enumerate() {
                        let x = 6.0 + i as f64 * 10.0;
                        let pulse = 0.8 + 0.2 * (dot.phase * std::f64::consts::TAU).sin();
                        ctx.arc(x, 12.0, 3.0 * pulse, 0.0, std::f64::consts::TAU);
                        ctx.set_source_rgb(dot.r, dot.g, dot.b);
                        ctx.fill().ok();
                    }
                    Propagation::Proceed
                });
                content.pack_end(&da, false, false, 2);
                Some(da)
            } else {
                None
            };

            button.add(&content);
            self.container.pack_start(&button, false, false, 0);

            buttons.push(ButtonState {
                button,
                icon,
                label,
                drawing_area,
                dots: dots_state,
            });
        }

        self.container.show_all();
    }

    // =========================================================================
    // STRATEGY 3: Animation only (just queue_draw on DrawingAreas)
    // =========================================================================
    fn render_animation_only(&self, dots: &[ProviderDot]) {
        let start = Instant::now();
        let buttons = self.buttons.borrow();

        if buttons.is_empty() {
            self.update_timing("ANIM ONLY", 0, 0, 0, 0, &[]);
            return;
        }

        let t0 = Instant::now();
        for bs in buttons.iter() {
            // Update animation state
            *bs.dots.borrow_mut() = dots.to_vec();
            // Queue redraw
            if let Some(ref da) = bs.drawing_area {
                da.queue_draw();
            }
        }
        let anim_us = t0.elapsed().as_micros();

        let total_us = start.elapsed().as_micros();
        self.update_timing("ANIM ONLY", total_us, 0, anim_us, 0, &[]);
    }

    fn update_timing(
        &self,
        strategy: &str,
        total_us: u128,
        clear_us: u128,
        main_us: u128,
        show_us: u128,
        per_widget: &[u128],
    ) {
        *self.render_count.borrow_mut() += 1;
        let count = *self.render_count.borrow();

        let avg_widget = if per_widget.is_empty() {
            0
        } else {
            per_widget.iter().sum::<u128>() / per_widget.len() as u128
        };

        let text = format!(
            "[{}] Render #{}\n\
             Total: {:.2}ms\n\
             Clear: {:.2}ms | Main: {:.2}ms | Show: {:.2}ms\n\
             Avg per widget: {:.2}μs",
            strategy,
            count,
            total_us as f64 / 1000.0,
            clear_us as f64 / 1000.0,
            main_us as f64 / 1000.0,
            show_us as f64 / 1000.0,
            avg_widget as f64,
        );
        self.timing_label.set_text(&text);

        println!(
            "[{} #{}] total={:.2}ms clear={:.2}ms main={:.2}ms show={:.2}ms avg_widget={:.0}μs",
            strategy,
            count,
            total_us as f64 / 1000.0,
            clear_us as f64 / 1000.0,
            main_us as f64 / 1000.0,
            show_us as f64 / 1000.0,
            avg_widget as f64,
        );
    }
}

fn main() {
    let app = Application::builder()
        .application_id("org.repro.workspace_widget")
        .build();

    app.connect_activate(|app| {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("GTK Render Strategy Comparison")
            .default_width(900)
            .default_height(300)
            .build();

        let main_box = GtkBox::new(gtk::Orientation::Vertical, 8);
        main_box.set_margin_top(8);
        main_box.set_margin_bottom(8);
        main_box.set_margin_start(8);
        main_box.set_margin_end(8);

        // Create 3 widgets for comparison
        let timing1 = Label::new(Some("Strategy 1: Full Rebuild"));
        let timing2 = Label::new(Some("Strategy 2: Reuse Widgets"));
        let timing3 = Label::new(Some("Strategy 3: Animation Only"));

        timing1.set_halign(gtk::Align::Start);
        timing2.set_halign(gtk::Align::Start);
        timing3.set_halign(gtk::Align::Start);

        let widget1 = Rc::new(WorkspaceWidget::new(timing1.clone()));
        let widget2 = Rc::new(WorkspaceWidget::new(timing2.clone()));
        let widget3 = Rc::new(WorkspaceWidget::new(timing3.clone()));

        // Test data
        let workspaces: Vec<WorkspaceInfo> = (0..NUM_WORKSPACES)
            .map(|i| WorkspaceInfo {
                number: i as i32,
                is_active: i == 3,
                window_count: if i % 3 == 0 { 2 } else { 0 },
            })
            .collect();

        // Animation phase - will be updated each frame
        let phase = Rc::new(RefCell::new(0.0f64));

        // Initial render for widget2 and widget3 (they need persistent state)
        let dots: Vec<ProviderDot> = vec![
            ProviderDot { r: 0.8, g: 0.6, b: 0.2, phase: 0.0 },
            ProviderDot { r: 0.5, g: 0.8, b: 0.5, phase: 0.0 },
        ];
        widget2.render_reuse(&workspaces, &dots);
        widget3.render_reuse(&workspaces, &dots); // Initialize widget3 too

        // Animation timer - 60fps
        {
            let widget1 = widget1.clone();
            let widget2 = widget2.clone();
            let widget3 = widget3.clone();
            let workspaces = workspaces.clone();
            let phase = phase.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                // Update animation phase
                let mut p = phase.borrow_mut();
                *p = (*p + 0.05) % 1.0;
                let current_phase = *p;
                drop(p);

                let dots: Vec<ProviderDot> = vec![
                    ProviderDot { r: 0.8, g: 0.6, b: 0.2, phase: current_phase },
                    ProviderDot { r: 0.5, g: 0.8, b: 0.5, phase: current_phase + 0.5 },
                ];

                // Test all 3 strategies
                widget1.render_full_rebuild(&workspaces, &dots);
                widget2.render_reuse(&workspaces, &dots);
                widget3.render_animation_only(&dots);

                glib::ControlFlow::Continue
            });
        }

        // Layout
        main_box.pack_start(&Label::new(Some("Strategy 1: FULL REBUILD (destroy + recreate all widgets)")), false, false, 0);
        main_box.pack_start(widget1.widget(), false, false, 0);
        main_box.pack_start(&timing1, false, false, 0);

        main_box.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 4);

        main_box.pack_start(&Label::new(Some("Strategy 2: REUSE (update classes + queue_draw)")), false, false, 0);
        main_box.pack_start(widget2.widget(), false, false, 0);
        main_box.pack_start(&timing2, false, false, 0);

        main_box.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 4);

        main_box.pack_start(&Label::new(Some("Strategy 3: ANIMATION ONLY (just queue_draw on DrawingAreas)")), false, false, 0);
        main_box.pack_start(widget3.widget(), false, false, 0);
        main_box.pack_start(&timing3, false, false, 0);

        window.add(&main_box);
        window.show_all();
    });

    app.run();
}
