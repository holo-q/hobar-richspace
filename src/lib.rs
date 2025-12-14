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
use tracing_subscriber::{prelude::*, EnvFilter};

/// Initialize logging with tracing ecosystem
///
/// Sink: journald (view with: journalctl -t richspace)
/// Level: RUST_LOG env var (default: info), e.g. RUST_LOG=richspace=debug
fn init_logging() {
    let env_filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("richspace=info"));

    let journald_layer = tracing_journald::layer()
        .expect("Failed to connect to journald")
        .with_syslog_identifier("richspace".to_string());

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(journald_layer);

    let _ = tracing::subscriber::set_global_default(subscriber);
}

/// Entry point called by XFCE panel via C shim
///
/// This function is exported with C linkage and called when the plugin loads.
/// Initializes GTK, wnck, and starts the application.
#[no_mangle]
pub extern "C" fn constructor(pointer: XfcePanelPluginPointer) {
    // Initialize logging first (before any other operations)
    init_logging();

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
