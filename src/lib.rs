//! Richspace - XFCE4 Panel Plugin
//!
//! Custom workspace display with configurable labels and icons.
//! Supports emoji, FontAwesome, Nerd Fonts, or any Unicode text.
//!
//! State is ephemeral - stored in $XDG_RUNTIME_DIR and resets on logout.
//! Configuration can be modified via JSON file with live reload.
//!
//! ## Tracing
//!
//! Comprehensive instrumentation for debugging freezes/lockups.
//! View logs: `journalctl -t richspace -f`
//!
//! Log levels:
//! - ERROR: Failures that break functionality
//! - WARN: Degraded operation, non-fatal issues
//! - INFO: Lifecycle events (start, stop, reload)
//! - DEBUG: Event flow, state changes
//! - TRACE: Hot path details (render, animation ticks)

mod xfce;
mod wnck;
mod config;
mod state;
mod app;
mod ui;
mod providers;

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
    spaceship_std::init_logging!("richspace", &spaceship_std::LoggingArgs::default());

    let start = std::time::Instant::now();
    tracing::info!("═══ richspace plugin constructor BEGIN ═══");

    // Initialize GTK
    tracing::debug!("initializing GTK");
    if gtk::init().is_err() {
        tracing::error!("FATAL: failed to initialize GTK");
        return;
    }
    tracing::debug!(elapsed_ms = start.elapsed().as_millis(), "GTK initialized");

    // Initialize wnck (must happen after GTK init)
    tracing::debug!("initializing wnck");
    wnck::init();
    tracing::debug!(elapsed_ms = start.elapsed().as_millis(), "wnck initialized");

    // Wrap the raw pointer
    tracing::debug!(pointer = ?pointer, "wrapping plugin pointer");
    let plugin = XfcePanelPlugin::from_raw(pointer);
    tracing::info!(
        plugin_id = plugin.id(),
        plugin_name = plugin.name(),
        "plugin wrapped"
    );

    // Start the application
    tracing::debug!("starting application");
    app::App::start(plugin);

    tracing::info!(
        total_ms = start.elapsed().as_millis(),
        "═══ richspace plugin constructor END ═══"
    );
}
