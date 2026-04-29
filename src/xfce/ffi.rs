//! Raw FFI bindings to libxfce4panel
//!
//! Minimal bindings - just enough to register as a panel plugin
//! and respond to panel events.

use glib_sys::gboolean;
use libc::{c_char, c_int};

/// Opaque pointer to XfcePanelPlugin widget
pub type XfcePanelPluginPointer = *mut gtk_sys::GtkWidget;

/// Panel screen position
#[allow(dead_code)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XfceScreenPosition {
    None,
    NorthWestHorizontal,
    North,
    NorthEastHorizontal,
    NorthWestVertical,
    West,
    SouthWestVertical,
    NorthEastVertical,
    East,
    SouthEastVertical,
    SouthWestHorizontal,
    South,
    SouthEastHorizontal,
    FloatingHorizontal,
    FloatingVertical,
}

/// Panel plugin mode
#[allow(dead_code)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XfcePanelPluginMode {
    Horizontal,
    Vertical,
    Deskbar,
}

#[link(name = "xfce4panel-2.0")]
#[allow(dead_code)]
extern "C" {
    // Plugin identity
    // NOTE: There is no xfce_panel_plugin_get_id - use get_name + get_unique_id
    // The "plugin-15" style ID must be constructed from name + unique_id
    pub fn xfce_panel_plugin_get_name(plugin: XfcePanelPluginPointer) -> *const c_char;
    pub fn xfce_panel_plugin_get_unique_id(plugin: XfcePanelPluginPointer) -> c_int;

    // Panel geometry
    pub fn xfce_panel_plugin_get_orientation(
        plugin: XfcePanelPluginPointer,
    ) -> gtk_sys::GtkOrientation;
    pub fn xfce_panel_plugin_get_screen_position(
        plugin: XfcePanelPluginPointer,
    ) -> XfceScreenPosition;
    pub fn xfce_panel_plugin_get_size(plugin: XfcePanelPluginPointer) -> c_int;
    pub fn xfce_panel_plugin_get_mode(plugin: XfcePanelPluginPointer) -> XfcePanelPluginMode;
    pub fn xfce_panel_plugin_get_icon_size(plugin: XfcePanelPluginPointer) -> c_int;

    // Menu integration
    pub fn xfce_panel_plugin_menu_show_configure(plugin: XfcePanelPluginPointer);
    pub fn xfce_panel_plugin_add_action_widget(
        plugin: XfcePanelPluginPointer,
        widget: *mut gtk_sys::GtkWidget,
    );

    // Configuration file paths
    pub fn xfce_panel_plugin_save_location(
        plugin: XfcePanelPluginPointer,
        create: gboolean,
    ) -> *mut c_char;
}
