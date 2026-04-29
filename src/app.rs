//! Application state machine
//!
//! Elm-inspired architecture with centralized state and message passing.
//! Handles workspace changes, state file watching, provider IPC, and UI updates.

use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Config;
use crate::providers::{self, ProviderEvent, ProviderRegistry};
use crate::state::State;
use crate::ui::WorkspaceWidget;
use crate::wnck::{self, WorkspaceInfo};
use crate::xfce::XfcePanelPlugin;

use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Application events
#[derive(Debug)]
pub enum AppEvent {
    /// Workspace list changed (created/destroyed)
    WorkspacesChanged,
    /// Active workspace changed
    ActiveWorkspaceChanged,
    /// State file changed (live reload)
    StateFileChanged,
    /// Config file changed (hot reload rules, icons, etc.)
    ConfigFileChanged,
    /// Provider IPC event (render update, connection, etc.)
    ProviderUpdate(ProviderEvent),
    /// Animation tick (16ms for 60fps when providers are animating)
    AnimationTick,
    /// Panel orientation changed
    OrientationChanged(gtk::Orientation),
    /// Panel size changed
    SizeChanged(i32),
    /// User clicked a workspace button
    WorkspaceClicked(i32),
    /// User scrolled mouse wheel on workspace widget
    ScrollWorkspace { delta: i32, wrap: bool },
    /// Set custom label for a workspace (None = clear)
    SetWorkspaceLabel {
        workspace: i32,
        label: Option<String>,
    },
    /// Set custom icon for a workspace (None = clear)
    SetWorkspaceIcon {
        workspace: i32,
        icon: Option<String>,
    },
    /// Clear all customizations for a workspace
    ClearWorkspaceCustomizations { workspace: i32 },
    /// Reorder: move active workspace left/right in display order
    /// direction: -1 = move left, +1 = move right
    ReorderActiveWorkspace { direction: i32 },
    /// Reorder: move workspace from one display position to another (drag-and-drop)
    ReorderWorkspace { from_pos: usize, to_pos: usize },
    /// Move a dragged tasklist/window-button window onto a workspace.
    MoveWindowToWorkspace { xid: u64, workspace: i32 },
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
    /// Provider registry - tracks connected providers and their render states
    pub providers: ProviderRegistry,
}

/// Main application
pub struct App {
    plugin: XfcePanelPlugin,
    app_state: Rc<RefCell<AppState>>,
    widget: WorkspaceWidget,
    tx: glib::Sender<AppEvent>,
    /// File watcher for state live reload (kept alive)
    _state_watcher: Option<RecommendedWatcher>,
    /// File watcher for config hot reload (kept alive)
    _config_watcher: Option<RecommendedWatcher>,
    /// Animation tick source (60fps when providers are animating)
    animation_source: Option<glib::SourceId>,
    /// Last time provider triggered a full render (for throttling).
    /// Seeded in the past so babel's initial payload is not swallowed by startup throttling.
    last_provider_render: Instant,
    /// Shutdown signal for background watcher threads
    /// Set to true on cleanup to gracefully stop file watchers before plugin unload
    shutdown: Arc<AtomicBool>,
}

impl App {
    /// Start the application
    ///
    /// Initialization sequence with comprehensive tracing for debugging.
    pub fn start(plugin: XfcePanelPlugin) {
        let start = Instant::now();
        tracing::info!("App::start BEGIN");

        // Load persistent config
        tracing::debug!("loading config");
        let config_path = plugin.config_path();
        let config = config_path
            .as_ref()
            .and_then(|p| {
                tracing::debug!(path = %p.display(), "loading config from path");
                Config::load(p).ok()
            })
            .unwrap_or_else(|| {
                tracing::warn!("using default config (no config file found)");
                Config::default()
            });
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            icon_rules = config.icon_rules.len(),
            "config loaded"
        );

        // Load ephemeral state
        tracing::debug!("loading ephemeral state");
        let state = State::load().unwrap_or_default();
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            workspaces_with_state = state.workspaces.len(),
            "state loaded"
        );

        // Get initial workspace info
        tracing::debug!("fetching initial workspace info");
        let workspaces = wnck::get_workspaces();
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            workspace_count = workspaces.len(),
            "workspaces fetched"
        );

        // Create application state
        tracing::debug!("creating application state");
        let app_state = Rc::new(RefCell::new(AppState {
            config,
            config_path,
            state,
            workspaces,
            orientation: plugin.orientation(),
            size: plugin.size(),
            providers: ProviderRegistry::new(),
        }));

        // Create message channel
        tracing::debug!("creating glib message channel");
        let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);

        // Create UI
        tracing::debug!("creating workspace widget");
        let widget = WorkspaceWidget::new(&app_state.borrow(), tx.clone());
        plugin.container.add(widget.widget());
        plugin.add_action_widget(widget.widget());
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            "widget created and added"
        );

        // Show configure in menu
        plugin.menu_show_configure();

        // Shutdown signal for background threads - set on cleanup to gracefully stop watchers
        let shutdown = Arc::new(AtomicBool::new(false));

        // Set up file watchers for live reload
        tracing::debug!("setting up file watchers");
        let state_watcher = Self::setup_state_watcher(tx.clone(), shutdown.clone());
        let config_watcher = Self::setup_config_watcher(
            tx.clone(),
            app_state.borrow().config_path.clone(),
            shutdown.clone(),
        );
        tracing::debug!(
            state_watcher = state_watcher.is_some(),
            config_watcher = config_watcher.is_some(),
            "file watchers configured"
        );

        // Start provider IPC listener (separate channel to wrap ProviderEvent in AppEvent)
        tracing::debug!("starting provider IPC listener");
        {
            let tx = tx.clone();
            let (provider_tx, provider_rx) =
                glib::MainContext::channel::<ProviderEvent>(glib::Priority::DEFAULT);

            // Bridge provider events to main event loop.
            // Reorder events short-circuit to AppEvent::ReorderActiveWorkspace
            // instead of going through the provider registry -- they're fire-and-forget
            // IPC commands from external scripts, not provider state updates.
            provider_rx.attach(None, move |event| {
                match event {
                    ProviderEvent::Reorder { direction } => {
                        tracing::debug!(
                            direction,
                            "provider IPC reorder -> AppEvent::ReorderActiveWorkspace"
                        );
                        tx.send(AppEvent::ReorderActiveWorkspace { direction }).ok();
                    }
                    other => {
                        tracing::trace!(event = ?other, "provider event bridged to main loop");
                        tx.send(AppEvent::ProviderUpdate(other)).ok();
                    }
                }
                glib::ControlFlow::Continue
            });

            // Start the listener (runs in background thread with tokio)
            providers::start_listener(provider_tx);
        }
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            "provider listener started"
        );

        // Create app
        tracing::debug!("creating App instance");
        let app = Rc::new(RefCell::new(App {
            plugin,
            app_state: app_state.clone(),
            widget,
            tx: tx.clone(),
            _state_watcher: state_watcher,
            _config_watcher: config_watcher,
            animation_source: None,
            last_provider_render: Instant::now() - Duration::from_millis(200),
            shutdown,
        }));

        // Connect wnck signals
        tracing::debug!("connecting wnck signals");
        {
            let tx = tx.clone();
            wnck::connect_active_workspace_changed(move || {
                tracing::debug!("wnck: active_workspace_changed signal");
                tx.send(AppEvent::ActiveWorkspaceChanged).ok();
            });
        }
        {
            let tx = tx.clone();
            wnck::connect_workspace_created(move || {
                tracing::debug!("wnck: workspace_created signal");
                tx.send(AppEvent::WorkspacesChanged).ok();
            });
        }
        {
            let tx = tx.clone();
            wnck::connect_workspace_destroyed(move || {
                tracing::debug!("wnck: workspace_destroyed signal");
                tx.send(AppEvent::WorkspacesChanged).ok();
            });
        }
        // NOTE: We intentionally DON'T listen to window_opened/window_closed signals.
        // These fire for every window map/unmap during workspace switches, causing
        // massive signal spam (50+ events per switch). Instead, we refresh window
        // info when active_workspace_changed fires - get_workspaces() fetches current
        // window state for all workspaces anyway.
        tracing::debug!("wnck signals connected");

        // Connect panel signals
        tracing::debug!("connecting panel signals");
        {
            let tx = tx.clone();
            app.borrow()
                .plugin
                .connect_orientation_changed(move |orientation| {
                    tracing::debug!(?orientation, "panel: orientation_changed signal");
                    tx.send(AppEvent::OrientationChanged(orientation)).ok();
                });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_size_changed(move |size| {
                tracing::debug!(size, "panel: size_changed signal");
                tx.send(AppEvent::SizeChanged(size)).ok();
                true
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_configure_plugin(move || {
                tracing::debug!("panel: configure_plugin signal");
                tx.send(AppEvent::Configure).ok();
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_save(move || {
                tracing::debug!("panel: save signal");
                tx.send(AppEvent::Save).ok();
            });
        }
        {
            let tx = tx.clone();
            app.borrow().plugin.connect_free_data(move || {
                tracing::info!("panel: free_data signal (plugin unloading)");
                tx.send(AppEvent::Free).ok();
            });
        }
        tracing::debug!("panel signals connected");

        // Set up event handler
        // Guard against reentrant calls - GTK draw callbacks can fire synchronously
        // within handle_event, causing RefCell panic if we're already borrowed
        tracing::debug!("attaching event handler to main context");
        {
            let app_ref = app.clone();
            rx.attach(None, move |event| {
                match app_ref.try_borrow_mut() {
                    Ok(mut app) => app.handle_event(event),
                    Err(_) => {
                        tracing::warn!("Skipped event {:?} - reentrant call detected", event);
                    }
                }
                glib::ControlFlow::Continue
            });
        }

        // Show everything
        tracing::debug!("showing all widgets");
        app.borrow().plugin.container.show_all();

        tracing::info!(
            total_ms = start.elapsed().as_millis(),
            "App::start END - event loop now running"
        );
    }

    /// Set up file watcher for state live reload
    ///
    /// Background thread checks shutdown flag every 100ms to allow graceful exit.
    fn setup_state_watcher(
        tx: glib::Sender<AppEvent>,
        shutdown: Arc<AtomicBool>,
    ) -> Option<RecommendedWatcher> {
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
            let poll_timeout = std::time::Duration::from_millis(100);

            loop {
                // Check shutdown signal
                if shutdown.load(Ordering::SeqCst) {
                    tracing::debug!("state watcher thread shutting down");
                    break;
                }

                // Use timeout recv to allow periodic shutdown checks
                let result = match event_rx.recv_timeout(poll_timeout) {
                    Ok(r) => r,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };

                match result {
                    Ok(event) => {
                        // Only care about modify/create events
                        if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            continue;
                        }

                        // Check if this event is for our state file
                        let is_state_file = event
                            .paths
                            .iter()
                            .any(|p| p.file_name() == state_path_clone.file_name());

                        if !is_state_file {
                            continue;
                        }

                        // Debounce
                        let now = std::time::Instant::now();
                        if now.duration_since(last_event) < debounce {
                            continue;
                        }
                        last_event = now;

                        // Send event to main thread (check shutdown before sending)
                        if shutdown.load(Ordering::SeqCst) {
                            break;
                        }
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

    /// Set up file watcher for config hot reload
    ///
    /// Watches the config TOML file for changes, enabling live editing of
    /// icon rules, macros, and display settings without panel restart.
    /// Background thread checks shutdown flag every 100ms to allow graceful exit.
    fn setup_config_watcher(
        tx: glib::Sender<AppEvent>,
        config_path: Option<PathBuf>,
        shutdown: Arc<AtomicBool>,
    ) -> Option<RecommendedWatcher> {
        let config_path = config_path?;

        // Get the TOML path (config might be stored as .toml)
        let toml_path = Config::toml_path(&config_path);
        let watch_path = toml_path.parent()?;

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
                tracing::error!("failed to create config file watcher: {}", e);
                return None;
            }
        };

        // Watch the parent directory
        if let Err(e) = watcher.watch(watch_path, RecursiveMode::NonRecursive) {
            tracing::error!("failed to watch config directory: {}", e);
            return None;
        }

        tracing::info!("watching {} for config changes", watch_path.display());

        // Spawn background thread to process events
        let toml_path_clone = toml_path.clone();
        std::thread::spawn(move || {
            let mut last_event = std::time::Instant::now();
            let debounce = std::time::Duration::from_millis(200); // Slightly longer debounce for config
            let poll_timeout = std::time::Duration::from_millis(100);

            loop {
                // Check shutdown signal
                if shutdown.load(Ordering::SeqCst) {
                    tracing::debug!("config watcher thread shutting down");
                    break;
                }

                // Use timeout recv to allow periodic shutdown checks
                let result = match event_rx.recv_timeout(poll_timeout) {
                    Ok(r) => r,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };

                match result {
                    Ok(event) => {
                        // Only care about modify/create events
                        if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                            continue;
                        }

                        // Check if this event is for our config file
                        let is_config_file = event
                            .paths
                            .iter()
                            .any(|p| p.file_name() == toml_path_clone.file_name());

                        if !is_config_file {
                            continue;
                        }

                        // Debounce
                        let now = std::time::Instant::now();
                        if now.duration_since(last_event) < debounce {
                            continue;
                        }
                        last_event = now;

                        // Send event to main thread (check shutdown before sending)
                        if shutdown.load(Ordering::SeqCst) {
                            break;
                        }
                        if let Err(e) = tx.send(AppEvent::ConfigFileChanged) {
                            tracing::error!("failed to send config change event: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("config file watch error: {}", e);
                    }
                }
            }
        });

        Some(watcher)
    }

    /// Handle an event
    ///
    /// Central event dispatcher with comprehensive timing instrumentation.
    /// All events are logged at appropriate levels:
    /// - TRACE: AnimationTick (60fps, very noisy)
    /// - DEBUG: Normal events with timing
    /// - WARN: Slow events (>50ms)
    /// - ERROR: Failures
    fn handle_event(&mut self, event: AppEvent) {
        let event_start = Instant::now();

        // Skip logging for high-frequency events (720/sec when provider animating)
        let is_frequent = matches!(event, AppEvent::AnimationTick | AppEvent::ProviderUpdate(_));
        if !is_frequent {
            tracing::debug!(event = ?event, "handle_event BEGIN");
        }

        match event {
            AppEvent::WorkspacesChanged => {
                // Refresh workspace info - full rebuild needed (workspace added/removed)
                tracing::info!(
                    trigger = "workspaces_changed",
                    "SIGNAL IN - workspace count changed"
                );
                let t0 = Instant::now();
                {
                    let mut state_borrow = self.app_state.borrow_mut();
                    state_borrow.workspaces = wnck::get_workspaces();
                    // Reconcile display order with new workspace set
                    let ws_numbers: Vec<i32> =
                        state_borrow.workspaces.iter().map(|w| w.number).collect();
                    state_borrow.state.reconcile_display_order(&ws_numbers);
                }
                let t1 = Instant::now();
                let ws_count = self.app_state.borrow().workspaces.len();
                self.widget.render(&self.app_state.borrow());
                let t2 = Instant::now();
                tracing::info!(
                    workspace_count = ws_count,
                    get_workspaces_us = t1.duration_since(t0).as_micros(),
                    render_us = t2.duration_since(t1).as_micros(),
                    "RENDER OUT - workspace count change handled"
                );
            }
            AppEvent::ActiveWorkspaceChanged => {
                // Light update - just refresh active state, no widget rebuild
                // This is the fast path for workspace switching
                tracing::info!(
                    trigger = "active_workspace_changed",
                    "SIGNAL IN - workspace switch"
                );
                let t0 = Instant::now();

                // Scope borrows carefully - GTK callbacks during update_active can trigger
                // nested events, causing RefCell panic if app_state is still borrowed
                let (active, t1) = {
                    let mut state = self.app_state.borrow_mut();
                    state.workspaces = wnck::get_workspaces();
                    let t1 = Instant::now();
                    let active = state
                        .workspaces
                        .iter()
                        .find(|ws| ws.is_active)
                        .map(|ws| ws.number);
                    (active, t1)
                }; // borrow_mut dropped here before update_active

                self.widget.update_active(&self.app_state.borrow());
                let t2 = Instant::now();
                tracing::info!(
                    trigger = "active_workspace_changed",
                    active_workspace = ?active,
                    get_workspaces_us = t1.duration_since(t0).as_micros(),
                    update_active_us = t2.duration_since(t1).as_micros(),
                    "RENDER OUT - active state updated"
                );
            }
            AppEvent::StateFileChanged => {
                // Reload state from file
                tracing::info!(
                    trigger = "state_file_changed",
                    "SIGNAL IN - state file modified"
                );
                match State::load() {
                    Ok(new_state) => {
                        let ws_count = new_state.workspaces.len();
                        self.app_state.borrow_mut().state = new_state;
                        let render_start = Instant::now();
                        self.widget.render(&self.app_state.borrow());
                        tracing::info!(
                            trigger = "state_file_changed",
                            workspaces_with_state = ws_count,
                            render_us = render_start.elapsed().as_micros(),
                            "RENDER OUT - state reloaded"
                        );
                    }
                    Err(e) => {
                        tracing::error!(trigger = "state_file_changed", error = %e, "failed to reload state file");
                    }
                }
            }
            AppEvent::ConfigFileChanged => {
                // Hot reload config (icon rules, macros, display settings)
                tracing::info!(
                    trigger = "config_file_changed",
                    "SIGNAL IN - config file modified"
                );
                if let Some(ref path) = self.app_state.borrow().config_path.clone() {
                    match Config::load(&path) {
                        Ok(new_config) => {
                            let rule_count = new_config.icon_rules.len();
                            self.app_state.borrow_mut().config = new_config;
                            // Refresh CSS first (config may change typography/padding)
                            self.widget.refresh_css(&self.app_state.borrow());
                            let render_start = Instant::now();
                            self.widget.render(&self.app_state.borrow());
                            tracing::info!(
                                trigger = "config_file_changed",
                                path = %path.display(),
                                icon_rules = rule_count,
                                render_us = render_start.elapsed().as_micros(),
                                "RENDER OUT - config reloaded"
                            );
                        }
                        Err(e) => {
                            tracing::error!(
                                trigger = "config_file_changed",
                                path = %path.display(),
                                error = %e,
                                "failed to reload config"
                            );
                        }
                    }
                }
            }
            AppEvent::ProviderUpdate(ref provider_event) => {
                // Update provider registry
                self.app_state
                    .borrow_mut()
                    .providers
                    .handle_event(provider_event.clone());

                // Throttle full renders to 5fps (200ms interval)
                // Provider dots require widget rebuild which is expensive (~50ms)
                // Users don't need more than 5fps for status dots
                let now = Instant::now();
                let elapsed = now.duration_since(self.last_provider_render);
                if elapsed.as_millis() >= 200 {
                    self.last_provider_render = now;
                    tracing::debug!(
                        trigger = "provider_update",
                        elapsed_since_last_ms = elapsed.as_millis(),
                        "SIGNAL IN - provider throttle tick (5fps)"
                    );
                    let render_start = Instant::now();
                    self.widget.render(&self.app_state.borrow());
                    tracing::debug!(
                        trigger = "provider_update",
                        render_us = render_start.elapsed().as_micros(),
                        "RENDER OUT - provider render complete"
                    );
                }

                // Start/stop animation tick based on provider state
                self.update_animation_tick();
            }
            AppEvent::AnimationTick => {
                // DISABLED: Animation tick is useless with current architecture
                // DrawingArea closures capture render state at creation time,
                // so queue_redraw() just repaints with stale data.
                // Provider render at 5fps is sufficient for status dots.
                //
                // TODO: To enable smooth animations:
                // 1. Store render state in Rc<RefCell<RenderState>>
                // 2. Have closures read from shared state on each draw
                // 3. Then queue_redraw() would show updated state
                self.stop_animation_tick();
            }
            AppEvent::OrientationChanged(orientation) => {
                tracing::debug!(?orientation, "orientation changed");
                self.app_state.borrow_mut().orientation = orientation;
                self.widget.set_orientation(orientation);
                // Rebuild positions for new axis (Fixed needs repositioning)
                self.widget.rebuild(&self.app_state.borrow());
            }
            AppEvent::SizeChanged(size) => {
                tracing::debug!(size, "size changed");
                self.app_state.borrow_mut().size = size;
            }
            AppEvent::WorkspaceClicked(num) => {
                tracing::debug!(workspace = num, "workspace clicked - switching");
                wnck::switch_to_workspace(num);
            }
            AppEvent::ScrollWorkspace { delta, wrap } => {
                let state = self.app_state.borrow();
                let current = wnck::active_workspace_number().unwrap_or(0);
                let count = state.workspaces.len();

                if count == 0 {
                    tracing::warn!("scroll ignored - no workspaces");
                    return;
                }

                let display_order = state.state.effective_display_order(count);

                // Find current workspace's display position
                let current_pos = display_order
                    .iter()
                    .position(|&n| n == current)
                    .unwrap_or(0) as i32;
                let count_i32 = count as i32;

                let mut next_pos = current_pos + delta;
                if wrap {
                    next_pos = next_pos.rem_euclid(count_i32);
                } else {
                    next_pos = next_pos.clamp(0, count_i32 - 1);
                }

                // Map display position back to workspace number
                let next_ws = display_order[next_pos as usize];

                tracing::debug!(
                    current_ws = current,
                    current_pos = current_pos,
                    delta,
                    next_pos = next_pos,
                    next_ws,
                    wrap,
                    "scroll workspace (display-order-aware)"
                );
                drop(state);
                wnck::switch_to_workspace(next_ws);
            }
            AppEvent::SetWorkspaceLabel {
                workspace,
                ref label,
            } => {
                tracing::debug!(workspace, label = ?label, "set workspace label");
                // Update ephemeral state with custom label
                self.app_state
                    .borrow_mut()
                    .state
                    .set_label(workspace, label.clone());
                // Save to disk (for external tools and persistence)
                if let Err(e) = self.app_state.borrow().state.save() {
                    tracing::error!(error = %e, "failed to save state");
                }
                // Render immediately (don't wait for file watcher)
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::SetWorkspaceIcon {
                workspace,
                ref icon,
            } => {
                tracing::debug!(workspace, icon = ?icon, "set workspace icon");
                // Update ephemeral state with custom icon
                self.app_state
                    .borrow_mut()
                    .state
                    .set_icon(workspace, icon.clone());
                // Save to disk
                if let Err(e) = self.app_state.borrow().state.save() {
                    tracing::error!(error = %e, "failed to save state");
                }
                // Render immediately
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::ClearWorkspaceCustomizations { workspace } => {
                tracing::debug!(workspace, "clear workspace customizations");
                // Clear all customizations for this workspace (revert to defaults)
                self.app_state.borrow_mut().state.clear(workspace);
                // Save to disk
                if let Err(e) = self.app_state.borrow().state.save() {
                    tracing::error!(error = %e, "failed to save state");
                }
                // Render immediately
                self.widget.render(&self.app_state.borrow());
            }
            AppEvent::ReorderActiveWorkspace { direction } => {
                tracing::info!(direction, "SIGNAL IN - reorder active workspace");
                // Clamp IPC direction to valid range — external callers could send any i32
                let direction = direction.clamp(-1, 1);
                if direction == 0 {
                    return;
                }
                let active = wnck::active_workspace_number().unwrap_or(0);
                let mut state = self.app_state.borrow_mut();
                let ws_count = state.workspaces.len();
                let true_reorder = state.config.true_reorder;

                // Ensure display order is initialized
                if state.state.display_order.len() != ws_count {
                    let ws_nums: Vec<i32> = state.workspaces.iter().map(|w| w.number).collect();
                    state.state.reconcile_display_order(&ws_nums);
                }

                // Find active workspace's display position
                if let Some(pos) = state.state.display_position_of(active) {
                    let new_pos_val = (pos as i32 + direction) as usize;
                    if new_pos_val >= ws_count {
                        tracing::debug!(direction, pos, "reorder clamped at boundary");
                    } else if true_reorder {
                        // True reorder: swap actual window contents between workspaces.
                        // Display order stays the same -- the windows themselves move,
                        // making the reorder visible to all apps, pagers, and m-N shortcuts.
                        let other_ws = state.state.display_order[new_pos_val];
                        // CRITICAL: Drop state borrow BEFORE window swap to avoid
                        // state/window divergence if the swap fails.
                        drop(state);
                        // Swap the actual X11 windows between workspaces FIRST
                        if wnck::swap_workspace_contents(active, other_ws) {
                            // Window swap succeeded — now safe to swap ephemeral state
                            let mut state = self.app_state.borrow_mut();
                            state.state.swap_ephemeral(active, other_ws);
                            if let Err(e) = state.state.save() {
                                tracing::error!(error = %e, "failed to save swapped ephemeral state");
                            }
                            drop(state);
                            // Switch to the target workspace so the user follows their windows
                            wnck::switch_to_workspace(other_ws);
                            tracing::info!(
                                direction,
                                active_ws = active,
                                target_ws = other_ws,
                                "RENDER OUT - true reorder (windows swapped)"
                            );
                        } else {
                            tracing::warn!(
                                active_ws = active,
                                target_ws = other_ws,
                                "true reorder aborted — window swap failed"
                            );
                        }
                        // Render will happen via ActiveWorkspaceChanged signal from the switch
                    } else {
                        // Virtual reorder: only change display order in richspace.
                        // Buttons rearrange visually but actual workspace numbers stay put.
                        if let Some(_new_pos) = state.state.swap_adjacent(pos, direction) {
                            if let Err(e) = state.state.save() {
                                tracing::error!(error = %e, "failed to save reordered state");
                            }
                            drop(state);
                            self.widget.reorder_animate(&self.app_state.borrow());
                            tracing::info!(
                                direction,
                                active_ws = active,
                                "RENDER OUT - virtual reorder (display order swapped)"
                            );
                        } else {
                            tracing::debug!(direction, pos, "virtual reorder clamped at boundary");
                        }
                    }
                } else {
                    tracing::warn!(active, "active workspace not found in display order");
                }
            }
            AppEvent::ReorderWorkspace { from_pos, to_pos } => {
                tracing::info!(from_pos, to_pos, "SIGNAL IN - reorder workspace (drag)");
                let mut state = self.app_state.borrow_mut();
                let ws_count = state.workspaces.len();
                let true_reorder = state.config.true_reorder;

                // Ensure display order is initialized
                if state.state.display_order.len() != ws_count {
                    let ws_nums: Vec<i32> = state.workspaces.iter().map(|w| w.number).collect();
                    state.state.reconcile_display_order(&ws_nums);
                }

                if true_reorder {
                    // True reorder: swap window contents between the two workspaces.
                    // Display order stays identity -- the actual windows move.
                    // For drag, from_pos and to_pos are display positions.
                    // Bounds check: DnD positions can be stale after workspace add/remove
                    if from_pos >= state.state.display_order.len()
                        || to_pos >= state.state.display_order.len()
                    {
                        tracing::warn!(
                            from_pos,
                            to_pos,
                            len = state.state.display_order.len(),
                            "DnD reorder out of bounds"
                        );
                        return;
                    }
                    let ws_from = state.state.display_order[from_pos];
                    let ws_to = state.state.display_order[to_pos];
                    // CRITICAL: Drop state borrow BEFORE window swap to avoid
                    // state/window divergence if the swap fails.
                    drop(state);
                    // Swap the actual X11 windows FIRST
                    if wnck::swap_workspace_contents(ws_from, ws_to) {
                        // Window swap succeeded — now safe to swap ephemeral state
                        let mut state = self.app_state.borrow_mut();
                        state.state.swap_ephemeral(ws_from, ws_to);
                        if let Err(e) = state.state.save() {
                            tracing::error!(error = %e, "failed to save swapped ephemeral state");
                        }
                        drop(state);
                        // Full render to reflect the swapped window contents
                        self.widget.render(&self.app_state.borrow());
                        tracing::info!(
                            from_pos,
                            to_pos,
                            ws_from,
                            ws_to,
                            "RENDER OUT - true reorder drag (windows swapped)"
                        );
                    } else {
                        tracing::warn!(
                            ws_from,
                            ws_to,
                            "true reorder drag aborted — window swap failed"
                        );
                    }
                } else {
                    // Virtual reorder: only change display order
                    state.state.reorder(from_pos, to_pos);
                    if let Err(e) = state.state.save() {
                        tracing::error!(error = %e, "failed to save reordered state");
                    }
                    drop(state);
                    self.widget.reorder_animate(&self.app_state.borrow());
                    tracing::info!(
                        from_pos,
                        to_pos,
                        "RENDER OUT - virtual reorder drag (display order changed)"
                    );
                }
            }
            AppEvent::MoveWindowToWorkspace { xid, workspace } => {
                tracing::info!(
                    xid,
                    workspace,
                    "SIGNAL IN - move dragged window to workspace"
                );
                if wnck::move_window_to_workspace(xid, workspace) {
                    {
                        let mut state = self.app_state.borrow_mut();
                        state.workspaces = wnck::get_workspaces();
                    }
                    self.widget.render(&self.app_state.borrow());
                    tracing::info!(xid, workspace, "RENDER OUT - dragged window moved");
                } else {
                    tracing::warn!(xid, workspace, "dragged window move failed");
                }
            }
            AppEvent::Configure => {
                tracing::info!("configure requested");
                self.show_config_dialog();
            }
            AppEvent::Save => {
                tracing::debug!("save requested");
                self.save_config();
            }
            AppEvent::Free => {
                tracing::info!("free requested - beginning cleanup");
                self.cleanup();
            }
        }

        // Post-event timing - skip for high-frequency events
        let elapsed = event_start.elapsed();
        if !is_frequent {
            if elapsed.as_millis() > 200 {
                tracing::error!(
                    event = ?event,
                    elapsed_ms = elapsed.as_millis(),
                    "CRITICAL: Event took >200ms - UI stutter"
                );
            } else if elapsed.as_millis() > 50 {
                tracing::warn!(
                    event = ?event,
                    elapsed_ms = elapsed.as_millis(),
                    "SLOW: Event took >50ms"
                );
            } else {
                tracing::debug!(elapsed_us = elapsed.as_micros(), "handle_event END");
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
    ///
    /// Signals background watcher threads to stop, preventing crash on plugin unload.
    fn cleanup(&mut self) {
        // Signal watcher threads to stop (they check this every 100ms)
        self.shutdown.store(true, Ordering::SeqCst);
        self.stop_animation_tick();
        self.save_config();
        tracing::info!("cleanup complete, watchers signaled to stop");
    }

    /// Start/stop animation tick based on provider state
    ///
    /// DISABLED: Animation tick is ineffective with current architecture because
    /// DrawingArea closures capture render state at creation time. queue_redraw()
    /// just repaints with stale data. Provider render throttle at 5fps is sufficient.
    ///
    /// TODO: To re-enable smooth animations:
    /// 1. Store render state in Rc<RefCell<RenderState>>
    /// 2. Have DrawingArea closures read from shared state on each draw
    fn update_animation_tick(&mut self) {
        // Don't start animation tick - it's useless and wastes CPU
        if self.animation_source.is_some() {
            self.stop_animation_tick();
        }
    }

    /// Stop the animation tick if running
    fn stop_animation_tick(&mut self) {
        if let Some(source) = self.animation_source.take() {
            source.remove();
            tracing::debug!("animation tick stopped");
        }
    }
}
