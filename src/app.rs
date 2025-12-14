//! Application state machine
//!
//! Elm-inspired architecture with centralized state and message passing.
//! Handles workspace changes, state file watching, and UI updates.

use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::xfce::XfcePanelPlugin;
use crate::config::Config;
use crate::state::State;
use crate::wnck::{self, WorkspaceInfo};
use crate::ui::WorkspaceWidget;

use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::mpsc::channel;
use std::path::PathBuf;

/// Application events
#[derive(Debug)]
pub enum AppEvent {
    /// Workspace list changed (created/destroyed)
    WorkspacesChanged,
    /// Active workspace changed
    ActiveWorkspaceChanged,
    /// Window opened/closed (affects window counts)
    WindowsChanged,
    /// State file changed (live reload)
    StateFileChanged,
    /// Panel orientation changed
    OrientationChanged(gtk::Orientation),
    /// Panel size changed
    SizeChanged(i32),
    /// User clicked a workspace button
    WorkspaceClicked(i32),
    /// User scrolled mouse wheel on workspace widget
    ScrollWorkspace { delta: i32, wrap: bool },
    /// Set custom label for a workspace (None = clear)
    SetWorkspaceLabel { workspace: i32, label: Option<String> },
    /// Set custom icon for a workspace (None = clear)
    SetWorkspaceIcon { workspace: i32, icon: Option<String> },
    /// Clear all customizations for a workspace
    ClearWorkspaceCustomizations { workspace: i32 },
    /// Open configuration dialog
    Configure,
    /// Save configuration
    Save,
    /// Cleanup and exit
    Free,
}

/// Application state
pub struct AppState {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub state: State,
    pub workspaces: Vec<WorkspaceInfo>,
    pub orientation: gtk::Orientation,
    pub size: i32,
}

/// Main application
pub struct App {
    plugin: XfcePanelPlugin,
    app_state: Rc<RefCell<AppState>>,
    widget: WorkspaceWidget,
    #[allow(dead_code)]
    tx: glib::Sender<AppEvent>,
    /// File watcher for state live reload (kept alive)
    _watcher: Option<RecommendedWatcher>,
}

impl App {
    /// Start the application
    pub fn start(plugin: XfcePanelPlugin) {
        // Load persistent config
        let config_path = plugin.config_path();
        let config = config_path
            .as_ref()
            .and_then(|p| Config::load(p).ok())
            .unwrap_or_default();

        // Load ephemeral state
        let state = State::load().unwrap_or_default();

        // Get initial workspace info
        let workspaces = wnck::get_workspaces();

        // Create application state
        let app_state = Rc::new(RefCell::new(AppState {
            config,
            config_path,
            state,
            workspaces,
            orientation: plugin.orientation(),
            size: plugin.size(),
        }));

        // Create message channel
        let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);

        // Create UI
        let widget = WorkspaceWidget::new(&app_state.borrow(), tx.clone());
        plugin.container.add(widget.widget());
        plugin.add_action_widget(widget.widget());

        // Show configure in menu
        plugin.menu_show_configure();

        // Set up file watcher for state live reload
        let watcher = Self::setup_state_watcher(tx.clone());

        // Create app
        let app = Rc::new(RefCell::new(App {
            plugin,
            app_state: app_state.clone(),
            widget,
            tx: tx.clone(),
            _watcher: watcher,
        }));

        // Connect wnck signals
        {
            let tx = tx.clone();
            wnck::connect_active_workspace_changed(move || {
                tx.send(AppEvent::ActiveWorkspaceChanged).ok();
            });
        }
        {
            let tx = tx.clone();
            wnck::connect_workspace_created(move || {
                tx.send(AppEvent::WorkspacesChanged).ok();
            });
        }
        {
            let tx = tx.clone();
            wnck::connect_workspace_destroyed(move || {
                tx.send(AppEvent::WorkspacesChanged).ok();
            });
        }
        {
            let tx = tx.clone();
            wnck::connect_window_opened(move || {
                tx.send(AppEvent::WindowsChanged).ok();
            });
        }
        {
            let tx = tx.clone();
            wnck::connect_window_closed(move || {
                tx.send(AppEvent::WindowsChanged).ok();
            });
        }

        // Connect panel signals
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_orientation_changed(move |orientation| {
                tx.send(AppEvent::OrientationChanged(orientation)).ok();
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_size_changed(move |size| {
                tx.send(AppEvent::SizeChanged(size)).ok();
                true
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_configure_plugin(move || {
                tx.send(AppEvent::Configure).ok();
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_save(move || {
                tx.send(AppEvent::Save).ok();
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_free_data(move || {
                tx.send(AppEvent::Free).ok();
            });
        }

        // Set up event handler
        {
            let app_ref = app.clone();
            rx.attach(None, move |event| {
                app_ref.borrow_mut().handle_event(event);
                glib::ControlFlow::Continue
            });
        }

        // Show everything
        app.borrow().plugin.container.show_all();
    }

    /// Set up file watcher for state live reload
    fn setup_state_watcher(tx: glib::Sender<AppEvent>) -> Option<RecommendedWatcher> {
        let state_path = State::state_path();

        // Create parent directory if it doesn't exist
        if let Some(parent) = state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Set up notify watcher
        let (event_tx, event_rx) = channel();
        let config = NotifyConfig::default();

        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                let _ = event_tx.send(res);
            },
            config,
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("failed to create file watcher: {}", e);
                return None;
            }
        };

        // Watch the parent directory (in case file doesn't exist yet)
        let watch_path = state_path.parent().unwrap_or(&state_path);
        if let Err(e) = watcher.watch(watch_path, RecursiveMode::NonRecursive) {
            tracing::error!("failed to watch state directory: {}", e);
            return None;
        }

        tracing::info!("watching {} for state changes", watch_path.display());

        // Spawn background thread to process events
        let state_path_clone = state_path.clone();
        std::thread::spawn(move || {
            // Simple debouncing with last event time
            let mut last_event = std::time::Instant::now();
            let debounce = std::time::Duration::from_millis(100);

            while let Ok(result) = event_rx.recv() {
                match result {
                    Ok(event) => {
                        // Only care about modify/create events
                        if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            continue;
                        }

                        // Check if this event is for our state file
                        let is_state_file = event.paths.iter().any(|p| {
                            p.file_name() == state_path_clone.file_name()
                        });

                        if !is_state_file {
                            continue;
                        }

                        // Debounce
                        let now = std::time::Instant::now();
                        if now.duration_since(last_event) < debounce {
                            continue;
                        }
                        last_event = now;

                        // Send event to main thread
                        if let Err(e) = tx.send(AppEvent::StateFileChanged) {
                            tracing::error!("failed to send state change event: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("file watch error: {}", e);
                    }
                }
            }
        });

        Some(watcher)
    }

    /// Handle an event
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::WorkspacesChanged | AppEvent::ActiveWorkspaceChanged | AppEvent::WindowsChanged => {
                // Refresh workspace info
                self.app_state.borrow_mut().workspaces = wnck::get_workspaces();
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::StateFileChanged => {
                // Reload state from file
                if let Ok(new_state) = State::load() {
                    self.app_state.borrow_mut().state = new_state;
                    self.widget.render(&self.app_state.borrow());
                    tracing::info!("state reloaded from file");
                }
            }
            AppEvent::OrientationChanged(orientation) => {
                self.app_state.borrow_mut().orientation = orientation;
                self.widget.set_orientation(orientation);
            }
            AppEvent::SizeChanged(size) => {
                self.app_state.borrow_mut().size = size;
            }
            AppEvent::WorkspaceClicked(num) => {
                wnck::switch_to_workspace(num);
            }
            AppEvent::ScrollWorkspace { delta, wrap } => {
                // Get current workspace and total count
                let current = wnck::active_workspace_number().unwrap_or(0);
                let count = self.app_state.borrow().workspaces.len() as i32;

                if count == 0 {
                    return;
                }

                // Calculate next workspace
                let mut next = current + delta;

                if wrap {
                    // Wrap around using proper modulo for negative numbers
                    next = next.rem_euclid(count);
                } else {
                    // Clamp to valid range
                    next = next.clamp(0, count - 1);
                }

                // Switch to the calculated workspace
                wnck::switch_to_workspace(next);
            }
            AppEvent::SetWorkspaceLabel { workspace, label } => {
                // Update ephemeral state with custom label
                self.app_state.borrow_mut().state.set_label(workspace, label);
                // Save to disk (for external tools and persistence)
                if let Err(e) = self.app_state.borrow().state.save() {
                    tracing::error!("failed to save state: {}", e);
                }
                // Render immediately (don't wait for file watcher)
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::SetWorkspaceIcon { workspace, icon } => {
                // Update ephemeral state with custom icon
                self.app_state.borrow_mut().state.set_icon(workspace, icon);
                // Save to disk
                if let Err(e) = self.app_state.borrow().state.save() {
                    tracing::error!("failed to save state: {}", e);
                }
                // Render immediately
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::ClearWorkspaceCustomizations { workspace } => {
                // Clear all customizations for this workspace (revert to defaults)
                self.app_state.borrow_mut().state.clear(workspace);
                // Save to disk
                if let Err(e) = self.app_state.borrow().state.save() {
                    tracing::error!("failed to save state: {}", e);
                }
                // Render immediately
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::Configure => {
                self.show_config_dialog();
            }
            AppEvent::Save => {
                self.save_config();
            }
            AppEvent::Free => {
                self.cleanup();
            }
        }
    }

    /// Show configuration dialog
    fn show_config_dialog(&mut self) {
        // TODO: Implement config dialog
        tracing::warn!("config dialog not yet implemented");
    }

    /// Save configuration
    fn save_config(&self) {
        let state = self.app_state.borrow();
        if let Some(ref path) = state.config_path {
            if let Err(e) = state.config.save(path) {
                tracing::error!("failed to save config: {}", e);
            }
        }
    }

    /// Cleanup before exit
    fn cleanup(&mut self) {
        self.save_config();
    }
}
