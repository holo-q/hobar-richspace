//! Workspace tracking via wnck-rs
//!
//! Re-exports from shared wnck-rs crate with richspace-specific helpers.

pub use wnck_rs::{set_client_type, ClientType, Screen};

/// Initialize wnck (must be called after GTK init)
pub fn init() {
    set_client_type(ClientType::Pager);
}

/// Workspace information for richspace display
#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    /// Workspace number (0-indexed)
    pub number: i32,
    /// Workspace name from WM
    pub name: String,
    /// Whether this is the active workspace
    pub is_active: bool,
    /// Number of windows on this workspace
    pub window_count: usize,
    /// WM_CLASS names of windows on this workspace (for icon rules)
    pub window_classes: Vec<String>,
}

/// Get information about all workspaces
///
/// Uses wnck's cached state rather than forcing X11 sync. This is safe because
/// wnck updates its cache when X11 events arrive - by the time signals fire,
/// the state is already current. Avoiding force_update() prevents GTK main thread
/// blocking during rapid X11 activity (e.g., window dragging).
pub fn get_workspaces() -> Vec<WorkspaceInfo> {
    let Some(screen) = Screen::get_default() else {
        return vec![];
    };

    // NOTE: Do NOT call screen.force_update() here!
    // force_update() does a synchronous X11 round-trip that blocks the GTK main thread.
    // When X11 is busy (e.g., during window drag), this can freeze the panel for 7-15 seconds.
    // wnck already updates its state via X11 event handlers before firing signals.

    let active_num = screen
        .active_workspace()
        .map(|ws| ws.get_number())
        .unwrap_or(-1);

    let windows = screen.get_windows();

    screen
        .get_workspaces()
        .into_iter()
        .map(|ws| {
            let number = ws.get_number();

            // Get windows on this workspace (excluding skip_tasklist)
            let ws_windows: Vec<_> = windows
                .iter()
                .filter(|w| {
                    w.get_workspace()
                        .map(|wws| wws.get_number() == number)
                        .unwrap_or(false)
                        && !w.is_skip_tasklist()
                })
                .collect();

            // Collect WM_CLASS names for icon rules
            let window_classes: Vec<String> = ws_windows
                .iter()
                .filter_map(|w| w.get_class_group())
                .collect();

            WorkspaceInfo {
                number,
                name: ws.get_name().unwrap_or_default(),
                is_active: number == active_num,
                window_count: ws_windows.len(),
                window_classes,
            }
        })
        .collect()
}

/// Get the active workspace number (0-indexed)
#[allow(dead_code)]
pub fn active_workspace_number() -> Option<i32> {
    Screen::get_default()?
        .active_workspace()
        .map(|ws| ws.get_number())
}

/// Switch to a workspace by number
pub fn switch_to_workspace(number: i32) {
    let Some(screen) = Screen::get_default() else { return };

    if let Some(ws) = screen.get_workspace(number) {
        ws.activate(0);
    }
}

/// Connect to workspace-changed signal (active workspace changed)
pub fn connect_active_workspace_changed<F: Fn() + 'static>(f: F) {
    if let Some(screen) = Screen::get_default() {
        screen.connect_active_workspace_changed(move |_| f());
    }
}

/// Connect to workspace-created signal
pub fn connect_workspace_created<F: Fn() + 'static>(f: F) {
    if let Some(screen) = Screen::get_default() {
        screen.connect_workspace_created(move |_| f());
    }
}

/// Connect to workspace-destroyed signal
pub fn connect_workspace_destroyed<F: Fn() + 'static>(f: F) {
    if let Some(screen) = Screen::get_default() {
        screen.connect_workspace_destroyed(move |_| f());
    }
}

/// Connect to window-opened signal (for updating window counts)
pub fn connect_window_opened<F: Fn() + 'static>(f: F) {
    if let Some(screen) = Screen::get_default() {
        screen.connect_window_opened(move |_| f());
    }
}

/// Connect to window-closed signal
pub fn connect_window_closed<F: Fn() + 'static>(f: F) {
    if let Some(screen) = Screen::get_default() {
        screen.connect_window_closed(move |_| f());
    }
}
