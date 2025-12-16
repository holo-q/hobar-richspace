//! Richspace - XFCE4 Panel Plugin
//!
//! Custom workspace display with configurable labels and icons.
//! Supports emoji, FontAwesome, Nerd Fonts, or any Unicode text.
//!
//! State is ephemeral - stored in $XDG_RUNTIME_DIR and resets on logout.
//! Configuration can be modified via JSON file with live reload.

mod xfce;
mod wnck;
mod config;
mod state;
mod app;
mod ui;

use xfce::{XfcePanelPluginPointer, XfcePanelPlugin};

/// Entry point called by XFCE panel via C shim
///
/// This function is exported with C linkage and called when the plugin loads.
/// Initializes logging, GTK, wnck, and starts the application.
///
/// Logging configuration:
/// - Sink: journald (view with: journalctl -t richspace)
/// - Level: Centralized ~/Workspace/logging.toml
/// - Hot-reload: pkill -HUP xfce4-panel (reloads all panel plugins' logging)
#[no_mangle]
pub extern "C" fn constructor(pointer: XfcePanelPluginPointer) {
    // Initialize logging first (before any other operations)
    // Uses centralized spaceship-std logging with journald sink and SIGHUP hot-reload
    spaceship_std::logging::init_simple("richspace", &spaceship_std::LoggingArgs::default());

    // Initialize GTK
    if gtk::init().is_err() {
        tracing::error!("Failed to initialize GTK");
        return;
    }

    // Initialize wnck (must happen after GTK init)
    wnck::init();

    // Wrap the raw pointer
    let plugin = XfcePanelPlugin::from_raw(pointer);

    // Start the application
    app::App::start(plugin);
}
